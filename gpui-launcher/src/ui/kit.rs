//! Building blocks that mirror the launcher's CSS components (.btn, .sq-btn, .pill, .chip, .switch, .dd …).
use super::theme::*;
use crate::text_input::TextInput;
use gpui::{
    anchored, deferred, div, img, point, prelude::*, px, Animation, AnimationExt, AnyElement,
    App, ClickEvent, Div, ElementId, Entity, FontWeight, Hsla, ImageSource, ObjectFit,
    SharedString, Stateful, Window,
};
use std::{rc::Rc, time::Duration};

pub type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
pub type PickHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

pub fn handler(f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Handler {
    Rc::new(f)
}

// ── icons ────────────────────────────────────────────────────────────────

/// A pixel icon from ui.js: rects on an 8×8 grid, snapped to whole pixels like `shape-rendering: crispEdges`.
/// "<name>-up" is the icon flipped vertically, "<name>@flipx" mirrored (CSS rotated them instead).
pub fn icon_parts(name: &str) -> (&str, bool, bool) {
    let (name, flip_x) = match name.strip_suffix("@flipx") {
        Some(b) => (b, true),
        None => (name, false),
    };
    let (base, flip) = match name.strip_suffix("-up") {
        Some(b) => (b, true),
        None => (name, false),
    };
    (base, flip, flip_x)
}

pub fn icon_rects(base: &str) -> &'static [(u8, u8, u8, u8)] {
    super::icons::ICONS.iter().find(|(n, _)| *n == base).map(|(_, r)| *r).unwrap_or(&[])
}

pub fn icon(name: &str, size: f32, color: impl Into<Hsla>) -> Div {
    let color = color.into();
    let unit = size / 8.0;
    let (base, flip, flip_x) = icon_parts(name);
    let rects = icon_rects(base);
    let mut d = div().relative().flex_none().w(px(size)).h(px(size));
    for &(x, y, w, h) in rects {
        let y = if flip { 8 - y - h } else { y };
        let x = if flip_x { 8 - x - w } else { x };
        let (l, t) = ((x as f32 * unit).round(), (y as f32 * unit).round());
        let (r, b) = (((x + w) as f32 * unit).round(), ((y + h) as f32 * unit).round());
        d = d.child(div().absolute().left(px(l)).top(px(t)).w(px(r - l)).h(px(b - t)).bg(color));
    }
    d
}

/// The Patchwork mark (assets/game/patchwork.png), tinted like `.pw-mark` (currentColor, white by default).
pub fn patchwork(size: f32) -> gpui::AnyElement {
    patchwork_tint(size, 0xf5f5f5)
}
pub fn patchwork_tint(size: f32, rgb: u32) -> gpui::AnyElement {
    match crate::assets::tinted("assets/game/patchwork.png", rgb) {
        Some(i) => img(i).w(px(size)).h(px(size)).flex_none().into_any_element(),
        None => div().size(px(size)).into_any_element(),
    }
}

pub fn pixel_text(text: impl Into<SharedString>, size: f32) -> Div {
    div().font_family(PIXEL).text_size(px(size)).line_height(px(size)).child(text.into())
}

pub fn mono(text: impl Into<SharedString>) -> Div {
    div().font_family(MONO).text_size(px(11.)).child(text.into())
}

/// `.spinner`: a 12px square that shows one quarter at a time, clockwise.
pub fn spinner(id: impl Into<ElementId>, color: impl Into<Hsla>) -> AnyElement {
    let color = color.into();
    if super::motion::reduced() { return div().size(px(12.)).flex_none().child(div().size(px(6.)).bg(color)).into_any_element(); }
    div()
        .relative()
        .size(px(12.))
        .flex_none()
        .child(div().absolute().size(px(6.)).bg(color))
        .with_animation(
            id,
            Animation::new(Duration::from_millis(900)).repeat(),
            move |el, t| {
                let q = ((t * 4.0).floor() as usize).min(3);
                let (l, top) = [(0., 0.), (6., 0.), (6., 6.), (0., 6.)][q];
                el.child(div().absolute().left(px(l)).top(px(top)).size(px(6.)).bg(color))
            },
        ).into_any_element()
}

// ── buttons ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Default,
    Primary,
    Dark,
    Danger,
    Stop,
}
#[derive(Clone, Copy, PartialEq)]
pub enum BSize {
    Sm,
    Md,
    Big,
    Tall,
}

/// `.btn` and its variants.
#[derive(IntoElement)]
pub struct Btn {
    id: ElementId,
    label: SharedString,
    icon: Option<&'static str>,
    icon_size: f32,
    kind: Kind,
    size: BSize,
    disabled: bool,
    full: bool,
    pixel: bool,
    danger_text: bool,
    min_w: Option<f32>,
    leading: Option<AnyElement>,
    trail: Option<&'static str>,
    on_click: Option<Handler>,
}

impl Btn {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), icon: None, icon_size: 16., kind: Kind::Default, size: BSize::Md, disabled: false, full: false, pixel: false, danger_text: false, min_w: None, leading: None, trail: None, on_click: None }
    }
    pub fn primary(mut self) -> Self { self.kind = Kind::Primary; self }
    pub fn dark(mut self) -> Self { self.kind = Kind::Dark; self }
    pub fn danger(mut self) -> Self { self.kind = Kind::Danger; self }
    pub fn stop(mut self) -> Self { self.kind = Kind::Stop; self }
    pub fn danger_text(mut self) -> Self { self.danger_text = true; self }
    pub fn sm(mut self) -> Self { self.size = BSize::Sm; self }
    pub fn big(mut self) -> Self { self.size = BSize::Big; self }
    pub fn tall(mut self) -> Self { self.size = BSize::Tall; self }
    pub fn full(mut self) -> Self { self.full = true; self }
    pub fn pixel(mut self) -> Self { self.pixel = true; self }
    pub fn min_w(mut self, w: f32) -> Self { self.min_w = Some(w); self }
    pub fn icon(mut self, name: &'static str) -> Self { self.icon = Some(name); self }
    pub fn icon_size(mut self, size: f32) -> Self { self.icon_size = size; self }
    pub fn leading(mut self, el: impl IntoElement) -> Self { self.leading = Some(el.into_any_element()); self }
    /// An icon after the label (12px), like `Add mods →`.
    pub fn trail(mut self, name: &'static str) -> Self { self.trail = Some(name); self }
    pub fn disabled(mut self, d: bool) -> Self { self.disabled = d; self }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Btn {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let stop = self.kind == Kind::Stop;
        let (h, padx, radius, fsize) = match self.size {
            BSize::Sm => (34., 12., 9., 12.),
            BSize::Big => (46., 22., 12., 14.),
            BSize::Tall => (48., 14., 12., 14.),
            BSize::Md if stop => (46., 20., 12., 14.),
            BSize::Md => (38., 14., 10., 13.),
        };
        let (bg, border, mut fg, hover) = match self.kind {
            Kind::Default => (None, Some(LINE2), FG, Some(0x111111)),
            Kind::Primary => (Some(ACCENT), None, INK, Some(0xe1e1e1)),
            Kind::Dark => (Some(0x111111), Some(LINE2), FG, Some(0x111111)),
            Kind::Danger | Kind::Stop => (Some(0x160808), Some(0x4a1d1d), DANGER, Some(0x111111)),
        };
        if self.danger_text {
            fg = DANGER;
        }
        let weight = if matches!(self.kind, Kind::Primary | Kind::Stop) { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM };
        let key: SharedString = format!("btn:{:?}", self.id).into();
        let lift = if self.pixel { super::motion::lift(&key, 1.) } else { 0. };
        let mut d = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .relative()
            .top(px(lift))
            .h(px(h))
            .px(px(padx))
            .rounded(px(radius))
            .text_size(px(fsize))
            .font_weight(weight)
            .text_color(c(fg))
            .whitespace_nowrap();
        if let Some(bg) = bg {
            d = d.bg(c(bg));
        }
        if let Some(b) = border {
            d = d.border_1().border_color(c(b));
        }
        if self.full {
            d = d.w_full();
        }
        if let Some(w) = self.min_w {
            d = d.min_w(px(w));
        }
        if self.disabled {
            d = d.opacity(0.45);
        } else {
            d = super::motion::hoverable(d, key.clone()).cursor_pointer().active(|s| s.opacity(0.85));
            if let Some(hv) = hover {
                d = d.hover(move |s| s.bg(c(hv)));
            }
            if let Some(f) = self.on_click {
                d = d.on_click(move |e, w, cx| f(e, w, cx));
            }
        }
        if let Some(lead) = self.leading {
            d = d.child(lead);
        }
        if let Some(name) = self.icon {
            d = d.child(super::motion::animated_icon(name, self.icon_size, c(fg), &key));
        }
        if !self.label.is_empty() {
            if self.pixel {
                d = d.child(pixel_text(self.label, 16.));
            } else {
                d = d.child(self.label);
            }
        }
        if let Some(name) = self.trail {
            d = d.child(super::motion::animated_icon(name, 12., c(fg), &key));
        }
        d
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SqKind {
    Default,
    Primary,
    Big,
    BigPinned,
    Dark,
}

/// `.sq-btn`: a square icon button.
pub fn sq_btn(id: impl Into<ElementId>, name: &'static str, kind: SqKind) -> Stateful<Div> {
    let id = id.into();
    let key: SharedString = format!("sq:{id:?}").into();
    let (size, radius) = if matches!(kind, SqKind::Big | SqKind::BigPinned) { (46., 12.) } else { (36., 9.) };
    let (bg, border, fg) = match kind {
        SqKind::Default => (None, Some(LINE2), 0xd4d4d4),
        SqKind::Primary => (Some(ACCENT), None, INK),
        SqKind::Big => (Some(0x000000), Some(0x333333), 0xd4d4d4),
        SqKind::BigPinned => (Some(0x000000), Some(ACCENT), ACCENT),
        SqKind::Dark => (Some(0x000000), Some(LINE2), 0xd4d4d4),
    };
    let hover = if kind == SqKind::Primary { 0xe1e1e1 } else { 0x111111 };
    let d = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(radius))
        .cursor_pointer()
        .hover(move |s| s.bg(c(hover)))
        .active(|s| s.opacity(0.85))
        .child(super::motion::animated_icon(name, 16., c(fg), &key));
    let mut d = super::motion::hoverable(d, key);
    if let Some(bg) = bg {
        d = d.bg(c(bg));
    }
    if let Some(b) = border {
        d = d.border_1().border_color(c(b));
    }
    d
}

/// `.icon-btn` (40px, or `.xs` 26px). `on` = the highlighted state.
pub fn icon_btn(id: impl Into<ElementId>, name: &'static str, xs: bool, on: bool) -> Stateful<Div> {
    let id = id.into();
    let key: SharedString = format!("ib:{id:?}").into();
    let (size, radius, isz) = if xs { (26., 7., 12.) } else { (40., 10., 16.) };
    let mut d = div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(radius))
        .cursor_pointer()
        .text_color(c(DIM))
        .hover(|s| s.bg(c(0x141414)));
    if on {
        d = d.bg(c(0x141414));
    }
    super::motion::hoverable(d, key.clone()).active(|s| s.opacity(0.85)).child(super::motion::animated_icon(name, isz, c(if on { 0xfafafa } else { DIM }), &key))
}

/// `.link`: plain text button.
pub fn link(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(13.))
        .text_color(c(0xd4d4d4))
        .cursor_pointer()
        .hover(|s| s.text_color(c(0xffffff)))
        .child(label.into())
}

/// `.pill` (or the smaller `.chips-row .pill` with `small`).
pub fn pill(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool, small: bool) -> Stateful<Div> {
    let (h, padx, r, fs) = if small { (30., 12., 8., 12.) } else { (34., 14., 9., 13.) };
    let mut d = div()
        .id(id)
        .flex()
        .items_center()
        .h(px(h))
        .px(px(padx))
        .rounded(px(r))
        .text_size(px(fs))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .border_1();
    if on {
        d = d.border_color(c(0x2a2a2a)).bg(c(0x161616)).text_color(c(0xfafafa));
    } else {
        d = d
            .border_color(if small { c(LINE) } else { ca(0x00000000) })
            .text_color(c(MUTED))
            .hover(|s| s.text_color(c(0xfafafa)));
    }
    d.child(label.into())
}

/// `.seg`: a row of pills in a tray.
pub fn seg() -> Div {
    div().flex().gap(px(4.)).p(px(4.)).border_1().border_color(c(0x1a1a1a)).rounded(px(12.)).bg(c(0x060606))
}

/// `.chip` / `.chip.dark`.
pub fn chip(text: impl Into<SharedString>, dark: bool) -> Div {
    let d = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(9.))
        .rounded(px(6.))
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(c(0xe5e5e5))
        .whitespace_nowrap();
    let d = if dark { d.bg(c(0x000000)).border_1().border_color(c(LINE2)) } else { d.bg(ca(0x000000b3)) };
    d.child(text.into())
}

/// `.tag`.
pub fn tag(text: impl Into<SharedString>) -> Div {
    div().flex().items_center().h(px(22.)).px(px(8.)).rounded(px(5.)).bg(c(0x111111)).font_family(MONO).text_size(px(11.)).text_color(c(MUTED)).child(text.into())
}

/// `.switch`.
pub fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let id = id.into();
    let t = super::motion::since_change(format!("sw:{id:?}"), if on { "1" } else { "0" }, 180.);
    let k = super::motion::cubic(0.3, 1.5, 0.5, 1., t);
    let (from, to) = if on { (3., 23.) } else { (23., 3.) };
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .rounded(px(4.))
        .border_1()
        .cursor_pointer()
        .border_color(c(if on { ACCENT } else { 0x2a2a2a }))
        .bg(c(if on { ACCENT } else { 0x141414 }))
        .child(
            div()
                .absolute()
                .top(px(3.))
                .left(px(from + (to - from) * k))
                .size(px(14.))
                .bg(c(if on { INK } else { DIM2 })),
        )
}

/// `.card`.
pub fn card() -> Div {
    div().flex().flex_col().gap(px(10.)).p(px(16.)).border_1().border_color(c(LINE)).rounded(px(14.)).bg(c(CARD))
}

/// `.empty-box`.
pub fn empty_box(text: impl Into<SharedString>) -> Div {
    div().p(px(24.)).border_1().border_dashed().border_color(c(LINE2)).rounded(px(14.)).text_color(c(DIM)).text_size(px(13.)).flex().justify_center().child(text.into())
}

/// `.sec-head` h2 (pixel 16px).
pub fn sec_title(text: impl Into<SharedString>) -> Div {
    pixel_text(text, 16.)
}

/// `.sq` accent square (8px, or 6px small), optionally pulsing.
pub fn sq(id: impl Into<ElementId>, small: bool, pulse: bool) -> AnyElement {
    let s = if small { 6. } else { 8. };
    let d = div().size(px(s)).flex_none().bg(c(ACCENT));
    if pulse && !super::motion::reduced() {
        d.with_animation(id, Animation::new(Duration::from_millis(1600)).repeat(), |el, t| {
            el.opacity(1.0 - 0.65 * (1.0 - (t * 2.0 - 1.0).abs()))
        })
        .into_any_element()
    } else {
        d.into_any_element()
    }
}

// ── inputs ───────────────────────────────────────────────────────────────

/// The `.field input` box around a TextInput.
pub fn input_box(input: &Entity<TextInput>, window: &Window, cx: &App, mono_font: bool) -> Div {
    let focused = input.read(cx).focus_handle_ref().is_focused(window);
    div()
        .h(px(42.))
        .px(px(12.))
        .flex()
        .items_center()
        .border_1()
        .border_color(c(if focused { 0x3a3a3a } else { 0x222222 }))
        .rounded(px(10.))
        .bg(c(FIELD))
        .text_color(c(FG))
        .font_family(if mono_font { MONO } else { SANS })
        .text_size(px(if mono_font { 12. } else { 14. }))
        .line_height(px(20.))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

/// `.field`: label above an input.
pub fn field(label: impl Into<SharedString>, input: impl IntoElement) -> Div {
    div().flex().flex_col().gap(px(6.)).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(c(MUTED)).child(label.into()).child(input)
}

/// `.search`.
pub fn search_box(input: &Entity<TextInput>, window: &Window, cx: &App, big: bool) -> Div {
    let focused = input.read(cx).focus_handle_ref().is_focused(window);
    div()
        .h(px(if big { 42. } else { 38. }))
        .px(px(if big { 14. } else { 12. }))
        .flex()
        .items_center()
        .gap(px(10.))
        .border_1()
        .border_color(c(if focused { 0x333333 } else { 0x1f1f1f }))
        .rounded(px(if big { 12. } else { 10. }))
        .bg(c(0x060606))
        .text_color(c(FG))
        .text_size(px(13.))
        .line_height(px(18.))
        .child(icon("search", 16., c(DIM)))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

// ── dropdown ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DdOpt {
    pub value: SharedString,
    pub label: SharedString,
    pub sub: Option<SharedString>,
    pub img: Option<ImageSource>,
    pub pixelated: bool,
}

impl DdOpt {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self { value: value.into(), label: label.into(), sub: None, img: None, pixelated: false }
    }
    pub fn sub(mut self, sub: impl Into<SharedString>) -> Self { self.sub = Some(sub.into()); self }
    pub fn img(mut self, img: impl Into<ImageSource>, pixelated: bool) -> Self {
        self.img = Some(img.into());
        self.pixelated = pixelated;
        self
    }
}

/// `.dd`: button plus a floating list. The open state lives in the Launcher (`dd_open`).
#[derive(IntoElement)]
pub struct Dropdown {
    id: SharedString,
    options: Vec<DdOpt>,
    value: SharedString,
    width: f32,
    height: f32,
    large: bool,
    open: bool,
    caption: Option<SharedString>,
    on_toggle: Option<Handler>,
    on_pick: Option<PickHandler>,
}

#[allow(dead_code)]
impl Dropdown {
    pub fn new(id: impl Into<SharedString>, options: Vec<DdOpt>, value: impl Into<SharedString>, open: bool) -> Self {
        Self { id: id.into(), options, value: value.into(), width: 240., height: 40., large: false, open, caption: None, on_toggle: None, on_pick: None }
    }
    pub fn width(mut self, w: f32) -> Self { self.width = w; self }
    pub fn height(mut self, h: f32) -> Self { self.height = h; self }
    pub fn large(mut self) -> Self { self.large = true; self.height = 56.; self }
    pub fn caption(mut self, c: impl Into<SharedString>) -> Self { self.caption = Some(c.into()); self }
    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self { self.on_toggle = Some(Rc::new(f)); self }
    pub fn on_pick(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self { self.on_pick = Some(Rc::new(f)); self }
}

fn dd_img(src: ImageSource, size: f32, radius: f32, pixelated: bool) -> impl IntoElement {
    img(src).size(px(size)).rounded(px(radius)).flex_none().object_fit(if pixelated { ObjectFit::Contain } else { ObjectFit::Cover })
}

impl RenderOnce for Dropdown {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let cur = self.options.iter().find(|o| o.value == self.value).or(self.options.first()).cloned();
        let cur = cur.unwrap_or_else(|| DdOpt::new("", ""));
        let has_img = cur.img.is_some();
        let toggle = self.on_toggle.clone();
        let hover_key: SharedString = format!("dd:{}", self.id).into();
        let flip = super::motion::since_change(format!("ddc:{}", self.id), if self.open { "1" } else { "0" }, 200.);
        let mut button = div()
            .id(SharedString::from(format!("{}-btn", self.id)))
            .relative()
            .w_full()
            .h(px(self.height))
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(if has_img { if self.large { 8. } else { 6. } } else if self.large { 14. } else { 12. }))
            .pr(px(if self.large { 14. } else { 12. }))
            .border_1()
            .rounded(px(if self.large { 12. } else { 10. }))
            .cursor_pointer()
            .text_color(c(FG));
        button = if self.large {
            button.border_color(c(if self.open { 0x3a3a3a } else { 0x0e0e0e })).bg(c(0x0e0e0e))
        } else {
            button.border_color(c(if self.open { 0x3a3a3a } else { 0x222222 })).bg(c(FIELD))
        };
        if let Some(t) = toggle {
            button = button.on_click(move |e, w, cx| t(e, w, cx));
        }
        if let Some(src) = cur.img.clone() {
            let s = if self.large { 40. } else { 26. };
            button = button.child(dd_img(src, s, if self.large { 8. } else { 5. }, cur.pixelated));
        }
        let mut text = div().flex().flex_col().gap(px(3.)).min_w_0().flex_1();
        if let Some(cap) = self.caption.clone() {
            text = text.child(div().text_size(px(10.)).text_color(c(0x6b6b6b)).child(cap.to_uppercase()));
        }
        text = text.child(
            div()
                .text_size(px(if self.large { 14. } else { 13. }))
                .font_weight(if self.large { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                .truncate()
                .child(cur.label.clone()),
        );
        if let Some(sub) = cur.sub.clone() {
            text = text.child(div().font_family(MONO).text_size(px(11.)).text_color(c(DIM)).truncate().child(sub));
        }
        // The chevron turns over when the list opens (.2s), and hops on hover.
        let chevron = if flip < 0.5 { if self.open { "chevron" } else { "chevron-up" } } else if self.open { "chevron-up" } else { "chevron" };
        let squash = (1. - (flip * 2. - 1.).abs()).max(0.);
        button = button.child(text).child(div().flex_none().h(px(16.)).flex().items_center().child(div().h(px(16. * (1. - squash * 0.9))).overflow_hidden().flex().items_center().child(super::motion::animated_icon(chevron, 16., c(0x8f8f8f), &hover_key))));
        button = super::motion::hoverable(button, hover_key);

        let mut root = div().flex().flex_col().w(px(self.width)).flex_none().child(button);
        if self.open {
            let viewport = window.viewport_size();
            let close = self.on_toggle.clone();
            let back = div()
                .id(SharedString::from(format!("{}-back", self.id)))
                .w(viewport.width)
                .h(viewport.height)
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, move |_, w, cx| {
                    if let Some(f) = &close {
                        f(&ClickEvent::default(), w, cx)
                    }
                });
            let exit = super::motion::dropdown_exit(&self.id).unwrap_or(0.);
            let entry = super::motion::enter(format!("dd-in:{}", self.id), 160., 0.);
            let mut list = div()
                .id(SharedString::from(format!("{}-list", self.id)))
                .occlude()
                .mt(px(6.))
                .w(px(self.width))
                .max_h(px(320.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(px(6.))
                .border_1()
                .border_color(c(LINE2))
                .rounded(px(12.))
                .bg(c(CARD));
            for (i, o) in self.options.iter().enumerate() {
                let on = o.value == cur.value;
                let tall = o.sub.is_some() || o.img.is_some();
                let pick = self.on_pick.clone();
                let value = o.value.clone();
                let mut item = div()
                    .id(SharedString::from(format!("{}-opt-{i}", self.id)))
                    .w_full()
                    .flex_none()
                    .min_h(px(if tall { 48. } else { 38. }))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x141414)))
                    .on_click(move |_, w, cx| {
                        if let Some(f) = &pick {
                            f(&value, w, cx)
                        }
                    });
                if on {
                    item = item.bg(c(0x141414));
                }
                if let Some(src) = o.img.clone() {
                    item = item.child(dd_img(src, 32., 6., o.pixelated));
                }
                let mut t = div().flex().flex_col().gap(px(3.)).min_w_0().flex_1().child(
                    div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).truncate().child(o.label.clone()),
                );
                if let Some(sub) = o.sub.clone() {
                    t = t.child(div().font_family(MONO).text_size(px(11.)).text_color(c(DIM)).truncate().child(sub));
                }
                item = item.child(t);
                if on {
                    item = item.child(icon("check", 16., c(ACCENT)));
                }
                // `.dd-list > *` rise in, 20ms apart.
                let k = super::motion::enter(format!("ddi:{}:{i}", self.id), 200., 20. * i.min(4) as f32);
                list = list.child(item.opacity(k).relative().top(px(8. * (1. - k))));
            }
            let list = list.opacity(entry * (1. - exit)).relative().top(px(6. * (1. - entry) - 5. * exit));
            root = root
                .child(deferred(anchored().position(point(px(0.), px(0.))).position_mode(gpui::AnchoredPositionMode::Window).child(back)).with_priority(1))
                .child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(list)).with_priority(2));
        }
        root
    }
}

/// A full-window transparent layer that closes a popover when clicked (`.pop-back`).
pub fn backdrop(id: impl Into<ElementId>, window: &Window, on_close: Handler) -> impl IntoElement {
    let viewport = window.viewport_size();
    deferred(
        anchored().position(point(px(0.), px(0.))).position_mode(gpui::AnchoredPositionMode::Window).child(
            div()
                .id(id)
                .w(viewport.width)
                .h(viewport.height)
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, move |_, w, cx| on_close(&ClickEvent::default(), w, cx)),
        ),
    )
    .with_priority(1)
}

/// A small text tooltip (the `title` attribute).
pub struct Tip(pub &'static str);
impl gpui::Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().px(px(8.)).py(px(4.)).rounded(px(6.)).border_1().border_color(c(LINE2)).bg(c(0x0c0c0c)).text_size(px(12.)).text_color(c(FG)).child(self.0)
    }
}
