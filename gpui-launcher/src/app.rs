//! The window shell (src/js/main.js): top bar, routing, inbox and account menus, toasts and dialogs.
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, local_accounts, util, Launcher};
use gpui::{AnimationExt, 
    anchored, deferred, div, img, prelude::*, px, AnyElement, Context, Corner, Entity, FontWeight,
    MouseButton, SharedString, Subscription, Window,
};
use serde_json::Value;
use std::{rc::Rc, time::{Duration, Instant}};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Home,
    Instances,
    Mods,
    Servers,
    Skins,
    Create,
    Settings,
    Signin,
}

const TABS: [(Route, &str, &str); 6] = [
    (Route::Home, "Home", "home"),
    (Route::Instances, "Instances", "grid"),
    (Route::Mods, "Mods", "chest"),
    (Route::Servers, "Servers", "server"),
    (Route::Skins, "Skins", "player"),
    (Route::Create, "Create", "hammer"),
];

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub born: std::time::Instant,
    pub id: u64,
    pub text: SharedString,
    pub kind: ToastKind,
}

pub enum MKind {
    Text(Entity<TextInput>),
    Area(Entity<TextInput>),
    Select(Vec<DdOpt>, SharedString),
}

pub struct MField {
    pub name: &'static str,
    pub label: SharedString,
    pub kind: MKind,
}

pub type ModalSubmit = Rc<dyn Fn(&mut Launcher, &[(&'static str, String)], &mut Window, &mut Context<Launcher>)>;
pub type ModalExtra = Rc<dyn Fn(&mut Launcher, &mut Window, &mut Context<Launcher>)>;

/// `modal()` from ui.js. Text fields are TextInputs; Enter submits, Escape cancels.
pub struct Modal {
    pub title: SharedString,
    pub text: Option<SharedString>,
    pub fields: Vec<MField>,
    pub confirm: SharedString,
    pub danger: bool,
    pub extra: Option<(SharedString, ModalExtra)>,
    pub on_submit: ModalSubmit,
    _subs: Vec<Subscription>,
}

/// One field of a dialog: `(name, label, value, placeholder)`; a `select` uses `options`.
pub struct FieldSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub value: String,
    pub placeholder: &'static str,
    pub options: Option<Vec<DdOpt>>,
    pub area: bool,
}

impl FieldSpec {
    pub fn text(name: &'static str, label: &'static str, value: impl Into<String>, placeholder: &'static str) -> Self {
        Self { name, label, value: value.into(), placeholder, options: None, area: false }
    }
    pub fn area(name: &'static str, label: &'static str, value: impl Into<String>, placeholder: &'static str) -> Self {
        Self { area: true, ..Self::text(name, label, value, placeholder) }
    }
    pub fn select(name: &'static str, label: &'static str, value: impl Into<String>, options: Vec<DdOpt>) -> Self {
        Self { name, label, value: value.into(), placeholder: "", options: Some(options), area: false }
    }
}

pub fn path_launcher_logo() -> std::path::PathBuf {
    assets::path("assets/game/launcher_logo.png")
}

/// Reserve the actual height of wrapped dialog copy; flex intrinsic sizing alone
/// can report one line while the text painter draws several.
fn modal_text_height(text: &gpui::SharedString, window: &Window) -> f32 {
    let width = (f32::from(window.viewport_size().width) - 48.).min(440.).max(1.);
    let inner = (width - 50.).max(1.);
    let mut style = window.text_style();
    style.font_family = SANS.into();
    let run = style.to_run(text.len());
    let lines = window.text_system().shape_text(text.clone(), px(13.), &[run], Some(px(inner)), None)
        .map(|lines| lines.iter().map(|line| line.wrap_boundaries.len() + 1).sum::<usize>())
        .unwrap_or_else(|_| text.lines().count().max(1));
    lines.max(1) as f32 * 19.5 + 2.
}

impl Launcher {
    // ── navigation ───────────────────────────────────────────────────────

    pub fn go(&mut self, route: Route, cx: &mut Context<Self>) {
        if self.route == route {
            return;
        }
        if self.route == Route::Signin {
            self.v.signin.pano.update(cx, |pano, _| pano.clear());
        }
        self.route = route;
        crate::ui::motion::reset("page:");
        self.top_open = None;
        self.dd_open = None;
        if let Some((old, _)) = self.dd_closing.take() { crate::ui::motion::dropdown_close_end(&old); }
        self.on_route(route, cx);
        cx.notify();
    }

    // ── toasts ───────────────────────────────────────────────────────────

    pub fn toast(&mut self, text: impl Into<SharedString>, kind: ToastKind, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toasts.push(Toast { born: std::time::Instant::now(), id, text: text.into(), kind });
        let ms = if kind == ToastKind::Error { 6400 } else { 3600 };
        let timer = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            timer.timer(Duration::from_millis(ms)).await;
            let _ = view.update(cx, |this, cx| {
                this.toasts.retain(|t| t.id != id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn info(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(text, ToastKind::Info, cx)
    }
    pub fn error(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(text, ToastKind::Error, cx)
    }

    // ── dialogs ──────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn open_modal(
        &mut self,
        title: impl Into<SharedString>,
        text: Option<&str>,
        fields: Vec<FieldSpec>,
        confirm: &str,
        danger: bool,
        extra: Option<(SharedString, ModalExtra)>,
        on_submit: ModalSubmit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut subs = Vec::new();
        let mut out = Vec::new();
        let mut first: Option<Entity<TextInput>> = None;
        for f in fields {
            let kind = if let Some(options) = f.options {
                MKind::Select(options, f.value.into())
            } else {
                let placeholder = f.placeholder;
                let value = f.value;
                let input = cx.new(|cx| {
                    let mut t = TextInput::with_placeholder(cx, placeholder);
                    t.set_text(value);
                    t
                });
                subs.push(cx.subscribe_in(&input, window, |this, _, ev: &TextEvent, window, cx| {
                    if matches!(ev, TextEvent::Submit) {
                        this.submit_modal(window, cx);
                    }
                }));
                first.get_or_insert_with(|| input.clone());
                if f.area { MKind::Area(input) } else { MKind::Text(input) }
            };
            out.push(MField { name: f.name, label: f.label.into(), kind });
        }
        if let Some(input) = first {
            input.read(cx).focus_handle_ref().focus(window);
        }
        crate::ui::motion::reset("modal");
        self.modal_closing = None;
        self.modal = Some(Modal {
            title: title.into(),
            text: text.map(|t| t.to_string().into()),
            fields: out,
            confirm: confirm.to_string().into(),
            danger,
            extra,
            on_submit,
            _subs: subs,
        });
        self.dd_open = None;
        if let Some((old, _)) = self.dd_closing.take() { crate::ui::motion::dropdown_close_end(&old); }
        cx.notify();
    }

    /// `confirmBox()`: a red confirm dialog without fields.
    pub fn confirm(
        &mut self,
        title: impl Into<SharedString>,
        text: &str,
        confirm: &str,
        on_yes: ModalExtra,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_modal(title, Some(text), vec![], confirm, true, None, Rc::new(move |this, _, w, cx| on_yes(this, w, cx)), window, cx);
    }

    fn close_modal(&mut self, cx: &mut Context<Self>) {
        if self.modal.is_none() || self.modal_closing.is_some() { return; }
        if crate::ui::motion::reduced() {
            self.modal = None;
            cx.notify();
            return;
        }
        let started = Instant::now();
        self.modal_closing = Some(started);
        let timer = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            timer.timer(Duration::from_millis(160)).await;
            let _ = view.update(cx, |this, cx| {
                if this.modal_closing == Some(started) {
                    this.modal = None;
                    this.modal_closing = None;
                    cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }

    pub fn submit_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_closing.is_some() { return; }
        let Some(modal) = self.modal.take() else { return };
        let values: Vec<(&'static str, String)> = modal
            .fields
            .iter()
            .map(|f| {
                let v = match &f.kind {
                    MKind::Text(i) | MKind::Area(i) => i.read(cx).value().to_string(),
                    MKind::Select(_, v) => v.to_string(),
                };
                (f.name, v)
            })
            .collect();
        (modal.on_submit)(self, &values, window, cx);
        cx.notify();
    }

    // ── accounts & avatars ───────────────────────────────────────────────

    /// (id, name, offline) of the account in use.
    pub fn active_account(&self) -> Option<(String, String, bool)> {
        let id = self.selected_account_id();
        self.accounts().into_iter().find(|a| a.0 == id).map(|a| (a.0, a.1, a.2))
    }

    pub fn active_skin_bytes(&self) -> (String, Vec<u8>, bool) {
        let id = self.store["activeSkin"].as_str().unwrap_or("").to_string();
        if let Some(skin) = self.store["skins"].as_array().into_iter().flatten().find(|s| s["id"].as_str() == Some(&id)) {
            if let Some(file) = skin["file"].as_str().and_then(|f| util::safe_name(f).ok()) {
                if let Some(bytes) = assets::read_cached(&crate::launcher_data().join("skins").join(file)) {
                    return (id, bytes, skin["model"].as_str() == Some("slim"));
                }
            }
        }
        ("__default".into(), assets::default_skin(), false)
    }

    /// The skin to show for an account: breakmine.com for online accounts, else the launcher skin in use.
    pub fn account_skin(&mut self, name: &str, offline: bool, cx: &mut Context<Self>) -> (String, Vec<u8>) {
        if !offline {
            match self.remote_skins.get(name) {
                Some(Some(bytes)) => return (format!("remote:{name}"), bytes.clone()),
                Some(None) => {}
                None => self.fetch_remote_skin(name.to_string(), cx),
            }
        }
        let (id, bytes, _) = self.active_skin_bytes();
        (id, bytes)
    }

    pub fn fetch_remote_skin(&mut self, name: String, cx: &mut Context<Self>) {
        let previous = self.remote_skins.get(&name).cloned().flatten();
        self.remote_skins.entry(name.clone()).or_insert(None);
        let api = self.store.pointer("/settings/apiUrl").and_then(Value::as_str).unwrap_or("https://api.breakmine.com").trim_end_matches('/').to_string();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let who = name.clone();
            let bytes = executor
                .spawn(async move {
                    let url = format!("{api}/skin/{}", url::form_urlencoded::byte_serialize(who.as_bytes()).collect::<String>());
                    let resp = ureq::get(&url).timeout(Duration::from_secs(8)).call().ok()?;
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 256 * 1024), &mut buf).ok()?;
                    util::png_size(&buf).filter(|(w, h)| *w == 64 && (*h == 32 || *h == 64)).map(|_| buf)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.remote_skins.insert(name, bytes.or(previous));
                cx.notify();
            });
        })
        .detach();
    }

    pub fn avatar(&mut self, acct: Option<(String, String, bool)>, size: f32, cx: &mut Context<Self>) -> AnyElement {
        let (key, bytes) = match &acct {
            Some((_, name, offline)) => self.account_skin(name, *offline, cx),
            None => ("__default".into(), assets::default_skin()),
        };
        let radius = if size >= 40. { 8. } else { 5. };
        match assets::skin_head(&key, &bytes, 8) {
            Some(head) => img(head).size(px(size)).rounded(px(radius)).flex_none().into_any_element(),
            None => div().size(px(size)).rounded(px(radius)).bg(c(0x1a1a1a)).flex_none().into_any_element(),
        }
    }

    pub fn pick_account(&mut self, id: &str, cx: &mut Context<Self>) {
        match local_accounts::select(&crate::gpui_accounts_path(), id) {
            Ok(prefs) => self.account_prefs = prefs,
            Err(e) => self.error(e, cx),
        }
        self.top_open = None;
        cx.notify();
    }

    pub fn save_store(&mut self) {
        if let Err(e) = crate::store::write_json(&crate::gpui_store_path(), &self.store) {
            self.notice = Some(format!("Couldn't save launcher settings: {e}"));
        }
    }

    // ── shell ────────────────────────────────────────────────────────────

    fn tab(&self, route: Route, label: &'static str, icon_name: &'static str, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.route == route;
        let pad = if narrow { 8. } else { 14. };
        let key: SharedString = format!("tab:{label}").into();
        let grow = crate::ui::motion::since_change(format!("tabbar:{label}"), if on { "1" } else { "0" }, 250.);
        let grow = if on { crate::ui::motion::cubic(0.3, 1.4, 0.5, 1., grow) } else { 0. };
        let fg = if on { 0xfafafa } else { DIM };
        crate::ui::motion::hoverable(div().id(label).debug_selector(move || format!("nav-{label}").into()), key.clone())
            .relative()
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(9.))
            .px(px(pad))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(c(fg))
            .cursor_pointer()
            .hover(|s| s.text_color(c(0xfafafa)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.go(route, cx)))
            .child(crate::ui::motion::animated_icon(icon_name, 16., c(fg), &key))
            .when(!narrow, |d| d.child(label))
            .child(
                div().absolute().left(px(pad)).right(px(pad)).bottom(px(-11.)).h(px(3.)).child(
                    div().absolute().top_0().bottom_0().left(gpui::relative((1. - grow) / 2.)).right(gpui::relative((1. - grow) / 2.)).when(on, |d| d.bg(c(ACCENT))),
                ),
            )
    }

    fn win_btn(id: &'static str, name: &'static str, close: bool) -> gpui::Stateful<gpui::Div> {
        let key: SharedString = format!("win:{id}").into();
        crate::ui::motion::hoverable(div().id(id), key.clone())
            .w(px(34.))
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(move |s| if close { s.bg(c(0x3a1010)) } else { s.bg(c(0x141414)) })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::ui::motion::animated_icon(name, 16., c(DIM), &key))
    }

    fn window_buttons(&self, with_max: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(Self::win_btn("win-min", "min", false).on_click(|_, w, _| w.minimize_window()))
            .when(with_max, |d| d.child(Self::win_btn("win-max", "max", false).on_click(|_, w, _| w.zoom_window())))
            .child(Self::win_btn("win-close", "close", true).on_click(cx.listener(|this, _, _, cx| {
                this.stop_game();
                cx.quit();
            })))
    }

    fn inbox(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let notes: Vec<Value> = self.store["notes"].as_array().cloned().unwrap_or_default();
        let mut pop = div()
            .id("inbox")
            .occlude()
            .w(px(340.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .mt(px(10.))
            .border_1()
            .border_color(c(LINE2))
            .rounded(px(14.))
            .bg(c(CARD))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(c(0x141414))
                    .child(pixel_text("Inbox", 16.))
                    .when(!notes.is_empty(), |d| {
                        d.child(div().ml_auto().child(Btn::new("mark-all", "Mark all read").sm().on_click(cx.listener(|this, _, _, cx| {
                            if let Some(n) = this.store["notes"].as_array_mut() {
                                for note in n {
                                    note["read"] = Value::Bool(true);
                                }
                            }
                            this.save_store();
                            cx.notify();
                        }))))
                    }),
            );
        let k = crate::ui::motion::cubic(0.2, 0.9, 0.3, 1.15, crate::ui::motion::enter("pop:bell", 160., 0.));
        pop = pop.opacity(k).mt(px(10. - 6. * (1. - k)));
        if notes.is_empty() {
            pop = pop.child(div().p(px(28.)).flex().justify_center().text_color(c(DIM)).text_size(px(13.)).child("You're all caught up."));
        }
        for (i, n) in notes.iter().enumerate() {
            let unread = !n["read"].as_bool().unwrap_or(false);
            let id = n["id"].as_str().unwrap_or("").to_string();
            pop = pop.child(
                div()
                    .flex()
                    .gap(px(10.))
                    .pl(px(14.))
                    .pr(px(10.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(c(0x101010))
                    .when(unread, |d| d.bg(c(FIELD)))
                    .child(div().size(px(6.)).mt(px(6.)).flex_none().when(unread, |d| d.bg(c(ACCENT))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .text_size(px(13.))
                            .child(n["text"].as_str().unwrap_or("").to_string())
                            .child(mono(ago(n["at"].as_u64().unwrap_or(0))).text_color(c(DIM2))),
                    )
                    .child(icon_btn(SharedString::from(format!("note-x-{i}")), "close", true, false).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(list) = this.store["notes"].as_array_mut() {
                            list.retain(|n| n["id"].as_str() != Some(&id));
                        }
                        this.save_store();
                        cx.notify();
                    }))),
            );
        }
        pop
    }

    fn account_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.selected_account_id();
        let accounts = self.accounts();
        let mut pop = div()
            .id("acct-menu")
            .occlude()
            .w(px(260.))
            .mt(px(10.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .border_1()
            .border_color(c(LINE2))
            .rounded(px(14.))
            .bg(c(CARD))
            .child(eyebrow("Accounts"));
        let k = crate::ui::motion::cubic(0.2, 0.9, 0.3, 1.15, crate::ui::motion::enter("pop:acct", 160., 0.));
        pop = pop.opacity(k).mt(px(10. - 6. * (1. - k)));
        for (id, name, offline, expired, _) in accounts {
            let on = id == active;
            let av = self.avatar(Some((id.clone(), name.clone(), offline)), 28., cx);
            let pick = id.clone();
            pop = pop.child(
                div()
                    .id(SharedString::from(format!("acct-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x111111)))
                    .when(on, |d| d.bg(c(0x111111)))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_account(&pick, cx)))
                    .child(av)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .text_size(px(13.))
                            .child(div().truncate().child(name))
                            .child(mono(if offline { "Offline" } else if expired { "breakmine.com · expired" } else { "breakmine.com" }).text_color(c(DIM))),
                    )
                    .when(on, |d| d.child(icon("check", 16., c(ACCENT)))),
            );
        }
        pop.child(div().h(px(1.)).my(px(4.)).bg(c(0x161616)))
            .child(menu_item("m-add", "Add account", false).on_click(cx.listener(|this, _, _, cx| this.go(Route::Signin, cx))))
            .child(menu_item("m-skin", "Change skin", false).on_click(cx.listener(|this, _, _, cx| this.go(Route::Skins, cx))))
            .when(!active.is_empty(), |d| {
                d.child(menu_item("m-out", "Sign out", true).on_click(cx.listener(|this, _, window, cx| {
                    this.top_open = None;
                    this.sign_out(window, cx);
                })))
            })
    }

    fn top_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unread = self.store["notes"].as_array().map(|n| n.iter().filter(|x| !x["read"].as_bool().unwrap_or(false)).count()).unwrap_or(0);
        let acct = self.active_account();
        let av = self.avatar(acct.clone(), 26., cx);
        let bell_open = self.top_open == Some("bell");
        let acct_open = self.top_open == Some("acct");
        // @media (max-width: 1100px)
        let width = f32::from(window.viewport_size().width);
        let narrow = width <= 1100.;
        let compact = width <= 1050.;
        let mut tabs = div().flex().gap(px(2.)).ml(px(if narrow { 4. } else { 10. }));
        for (route, label, ic) in TABS {
            tabs = tabs.child(self.tab(route, label, ic, compact, cx));
        }
        let close: Handler = handler({
            let view = cx.entity().downgrade();
            move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.top_open = None;
                    cx.notify();
                });
            }
        });

        let bell = div()
            .relative()
            .child(
                icon_btn("bell", "bell", false, bell_open)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.top_open = if this.top_open == Some("bell") { None } else { Some("bell") };
                        crate::ui::motion::reset("pop:");
                        cx.notify();
                    }))
                    .when(unread > 0, |d| d.child(div().absolute().top(px(9.)).right(px(9.)).size(px(6.)).bg(c(ACCENT)))),
            )
            .when(bell_open, |d| {
                d.child(backdrop("bell-back", window, close.clone()))
                    .child(div().absolute().top(px(40.)).right_0().child(deferred(anchored().anchor(Corner::TopRight).child(self.inbox(cx))).with_priority(2)))
            });

        let account = div()
            .relative()
            .ml(px(4.))
            .child(
                div()
                    .debug_selector(|| "top-account".into())
                    .id("acct-btn")
                    .h(px(40.))
                    .max_w(px(200.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(6.))
                    .pr(px(8.))
                    .border_1()
                    .border_color(c(0x1f1f1f))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x0e0e0e)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.top_open = if this.top_open == Some("acct") { None } else { Some("acct") };
                        crate::ui::motion::reset("pop:");
                        cx.notify();
                    }))
                    .child(av)
                    .when(!compact, |d| d.child(div().truncate().child(acct.as_ref().map(|a| a.1.clone()).unwrap_or_else(|| "Sign in".into()))))
                    .when(!compact, |d| d.child(icon(if acct_open { "chevron-up" } else { "chevron" }, 16., c(0x8f8f8f)))),
            )
            .when(acct_open, |d| {
                d.child(backdrop("acct-back", window, close.clone()))
                    .child(div().absolute().top(px(40.)).right_0().child(deferred(anchored().anchor(Corner::TopRight).child(self.account_menu(cx))).with_priority(2)))
            });

        let settings_on = self.route == Route::Settings;
        div()
            .id("topbar")
            .flex_none()
            .min_h(px(64.))
            .flex()
            .items_center()
            .gap(px(if compact { 2. } else if narrow { 8. } else { 18. }))
            .pl(px(if compact { 8. } else if narrow { 16. } else { 24. }))
            .pr(px(if compact { 6. } else { 14. }))
            .py(px(10.))
            .border_b_1()
            .border_color(c(0x141414))
            .bg(c(0x000000))
            .on_mouse_down(MouseButton::Left, |_, w, _| w.start_window_move())
            .child(
                div()
                    .id("brand")
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.go(Route::Home, cx)))
                    .child(img(path_launcher_logo()).w(px(if compact { 65. } else if narrow { 120. } else { 138. })).h(px(38.)).object_fit(gpui::ObjectFit::Contain).relative().top(px(12.))),
            )
            .child(tabs)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(bell)
                    .child(
                        icon_btn("gear", "gear", false, settings_on)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.go(Route::Settings, cx))),
                    )
                    .child(account)
                    .when(!compact, |d| d.child(div().w(px(1.)).h(px(22.)).mx(px(8.)).bg(c(0x1f1f1f))))
                    .when(!compact, |d| d.child(self.window_buttons(true, cx)))
                    .when(compact, |d| d.child(Self::win_btn("win-close-compact", "close", true).on_click(cx.listener(|this, _, _, cx| {
                        this.stop_game();
                        cx.quit();
                    })))),
            )
    }

    fn toasts_layer(&self) -> impl IntoElement {
        let mut col = div().absolute().right(px(20.)).bottom(px(20.)).flex().flex_col().items_end().gap(px(8.));
        for t in &self.toasts {
            let life = if t.kind == ToastKind::Error { 6400. } else { 3600. };
            let left = life - t.born.elapsed().as_secs_f32() * 1000.;
            let out = if crate::ui::motion::reduced() { 1. } else { (left / 300.).clamp(0., 1.) };
            if out < 1. {
                crate::ui::motion::request_frame();
            }
            let (glyph, fg, bg) = match t.kind {
                ToastKind::Error => ("times", DANGER, 0x281818),
                ToastKind::Success => ("check", ACCENT, 0x1b1b1b),
                ToastKind::Info => ("info", ACCENT, 0x1b1b1b),
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(11.))
                    .min_w(px(200.))
                    .max_w(px(380.))
                    .pl(px(10.))
                    .pr(px(14.))
                    .py(px(10.))
                    .border_1()
                    .border_color(c(0x292929))
                    .rounded(px(12.))
                    .bg(c(0x0c0c0c))
                    .text_size(px(13.))
                    .line_height(gpui::relative(1.45))
                    .shadow(vec![gpui::BoxShadow { color: ca(0x00000080), offset: gpui::point(px(0.), px(12.)), blur_radius: px(32.), spread_radius: px(0.) }])
                    .child(div().size(px(30.)).flex_none().flex().items_center().justify_center().rounded(px(8.)).bg(c(bg)).child(icon(glyph, 14., c(fg))))
                    .child(div().min_w_0().child(t.text.clone()))
                    .opacity(out)
                    .relative()
                    .top(px(8. * (1. - out)))
                    .with_animation(SharedString::from(format!("toast-{}", t.id)), gpui::Animation::new(Duration::from_millis(220)), |el, t| el.opacity(t).mt(px(8. * (1. - t)))),
            );
        }
        col
    }

    fn modal_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let modal = self.modal.as_ref()?;
        let max_height = (f32::from(window.viewport_size().height) - 48.).max(1.);
        let text_height = modal.text.as_ref().map(|text| modal_text_height(text, window));
        let minimum = (50. + 20. + 14. + text_height.map(|h| h + 14.).unwrap_or(0.) + 6. + 38.).min(max_height);
        let card = div()
            .id("modal-card")
            .debug_selector(|| "modal-card".into())
            .min_h(px(minimum))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .occlude()
            .w_full()
            .max_w(px(440.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .border_1()
            .border_color(c(LINE2))
            .rounded(px(18.))
            .bg(c(CARD))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(pixel_text(modal.title.clone(), 20.));
        let k = crate::ui::motion::cubic(0.2, 0.9, 0.3, 1.2, crate::ui::motion::enter("modal", 240., 0.));
        let exit = self.modal_closing.map(|at| (at.elapsed().as_secs_f32() / 0.16).clamp(0., 1.)).unwrap_or(0.);
        if self.modal_closing.is_some() && exit < 1. { crate::ui::motion::request_frame(); }
        let mut card = card.opacity((k * (1. - exit)).clamp(0., 1.)).relative().top(px(14. * (1. - k) + 10. * exit));
        if let Some(text) = &modal.text {
            card = card.child(div().w_full().flex_none().h(px(text_height.unwrap_or(21.5))).text_size(px(13.)).line_height(gpui::relative(1.5)).text_color(c(DIM)).child(text.clone()));
        }
        for f in &modal.fields {
            let el: AnyElement = match &f.kind {
                MKind::Text(input) => input_box(input, window, cx, false).into_any_element(),
                MKind::Area(input) => input_box(input, window, cx, false).h(px(84.)).items_start().pt(px(10.)).into_any_element(),
                MKind::Select(options, value) => {
                    let name = f.name;
                    let dd_id: SharedString = format!("mdd-{name}").into();
                    let open = self.dd_open.as_ref() == Some(&dd_id);
                    let toggle_id = dd_id.clone();
                    Dropdown::new(dd_id, options.clone(), value.clone(), open)
                        .width(392.)
                        .on_toggle(cx.listener(move |this, _, _, cx| {
                            this.toggle_dd(&toggle_id, cx);
                        }))
                        .on_pick(cx.listener(move |this, v: &SharedString, _, cx| {
                            if let Some(m) = &mut this.modal {
                                for f in &mut m.fields {
                                    if f.name == name {
                                        if let MKind::Select(_, val) = &mut f.kind {
                                            *val = v.clone();
                                        }
                                    }
                                }
                            }
                            this.dd_open = None;
                            cx.notify();
                        }))
                        .into_any_element()
                }
            };
            card = card.child(field(f.label.clone(), el));
        }
        let danger = modal.danger;
        let mut actions = div().debug_selector(|| "modal-actions".into()).flex().flex_none().min_h(px(38.)).items_center().justify_end().gap(px(8.)).mt(px(6.));
        if let Some((label, _)) = &modal.extra {
            actions = actions.child(
                div().mr_auto().child(Btn::new("m-extra", label.clone()).danger_text().on_click(cx.listener(|this, _, window, cx| {
                    if let Some(m) = this.modal.take() {
                        if let Some((_, f)) = m.extra {
                            f(this, window, cx);
                        }
                    }
                    cx.notify();
                }))),
            );
        }
        actions = actions
            .child(Btn::new("m-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                this.close_modal(cx);
            })))
            .child({
                let b = Btn::new("m-ok", modal.confirm.clone());
                let b = if danger { b.danger() } else { b.primary() };
                b.on_click(cx.listener(|this, _, window, cx| this.submit_modal(window, cx)))
            });
        card = card.child(actions);
        let backdrop_enter = crate::ui::motion::enter("modal-back", 160., 0.);
        Some(
            div()
                .id("modal-back")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .bg(ca(0x000000c7))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.close_modal(cx);
                }))
                .child(card)
                .opacity(backdrop_enter * (1. - exit)),
        )
    }

    pub fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if std::mem::take(&mut self.minimize_pending) {
            window.minimize_window();
        }
        if let Some(n) = self.notice.take() {
            let bad = ["Couldn't", "couldn't", "failed", "Failed", "error", "Error", "unexpectedly", "Can't", "can't", "isn't", "not "]
                .iter()
                .any(|w| n.contains(w));
            self.toast(n, if bad { ToastKind::Error } else { ToastKind::Info }, cx);
        }
        let signin = self.route == Route::Signin;
        let chrome: AnyElement = if signin {
            div()
                .absolute()
                .top_0()
                .right_0()
                .p(px(10.))
                .flex()
                .gap(px(4.))
                .child(Self::win_btn("s-min", "min", false).on_click(|_, w, _| w.minimize_window()))
                .child(Self::win_btn("s-close", "close", true).on_click(cx.listener(|this, _, _, cx| {
                    this.stop_game();
                    cx.quit();
                })))
                .into_any_element()
        } else {
            self.top_bar(window, cx).into_any_element()
        };
        crate::ui::motion::set_reduced(self.store.pointer("/settings/reduceMotion").and_then(serde_json::Value::as_bool).unwrap_or(false));
        let view = self.render_route(window, cx);
        let page_k = crate::ui::motion::cubic(0.2, 0.8, 0.3, 1., crate::ui::motion::enter(format!("page:{:?}", self.route), 140., 0.));
        let modal = self.modal_layer(window, cx).map(|m| m.into_any_element());
        let root = div()
            .id("root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(BG))
            .text_color(c(FG))
            .font_family(SANS)
            .text_size(px(14.))
            .line_height(gpui::relative(1.4))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    if let Some(id) = this.dd_open.clone() {
                        this.toggle_dd(&id, cx);
                    } else if this.top_open.take().is_some() {
                        cx.notify();
                    } else if this.modal.is_some() {
                        this.close_modal(cx);
                    } else {
                        this.on_escape(cx);
                    }
                }
            }))
            .when(!signin, |d| d.child(chrome))
            .child(div().id("view").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().child(div().flex_1().flex().flex_col().opacity(page_k).relative().top(px(10. * (1. - page_k))).child(view)))
            .when(signin, |d| {
                // The sign-in screen draws its own background; the window buttons float over it.
                d
            })
            .children(if signin { Some(self.signin_chrome(cx)) } else { None })
            .child(self.toasts_layer())
            .children(modal);
        // Anything still moving (icons, toasts, switches, lifts) asks for another frame.
        crate::ui::motion::flush(window);
        root.into_any_element()
    }

    fn signin_chrome(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(52.))
            .flex()
            .justify_end()
            .items_center()
            .px(px(10.))
            .on_mouse_down(MouseButton::Left, |_, w, _| w.start_window_move())
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(Self::win_btn("s-min", "min", false).on_click(|_, w, _| w.minimize_window()))
                    .child(Self::win_btn("s-close", "close", true).on_click(cx.listener(|this, _, _, cx| {
                        this.stop_game();
                        cx.quit();
                    }))),
            )
            .into_any_element()
    }
}

/// `.eyebrow`.
pub fn eyebrow(text: &'static str) -> impl IntoElement {
    div().pt(px(8.)).px(px(10.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(DIM2)).child(text.to_uppercase())
}

/// `.menu-item`.
pub fn menu_item(id: &'static str, label: &'static str, danger: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(10.))
        .py(px(9.))
        .rounded(px(9.))
        .text_size(px(13.))
        .cursor_pointer()
        .text_color(c(if danger { DANGER } else { FG }))
        .hover(|s| s.bg(c(0x111111)))
        .child(label)
}

/// `ago()` from ui.js.
pub fn ago(secs: u64) -> String {
    if secs == 0 {
        return "Never".into();
    }
    let d = util::now().saturating_sub(secs);
    if d < 60 {
        "Just now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else if d < 172800 {
        "Yesterday".into()
    } else if d < 604800 {
        format!("{}d ago", d / 86400)
    } else if d < 2592000 {
        format!("{}w ago", d / 604800)
    } else {
        // toLocaleDateString(): a plain date.
        let days = secs / 86400;
        let (y, m, dd) = civil(days as i64);
        format!("{m}/{dd}/{y}")
    }
}

/// Days since 1970-01-01 → (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// `playtime()` from ui.js.
pub fn playtime(secs: u64) -> String {
    if secs == 0 {
        "0h".into()
    } else if secs < 3600 {
        format!("{}m", ((secs as f64 / 60.).round() as u64).max(1))
    } else {
        format!("{}h", (secs as f64 / 3600.).round() as u64)
    }
}

/// `plural()` from ui.js.
pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// `bytes()` from ui.js.
pub fn bytes(n: u64) -> String {
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n < 1_048_576 {
        format!("{:.0} KB", f / 1024.)
    } else if n < 1_073_741_824 {
        if n < 10_485_760 { format!("{:.1} MB", f / 1_048_576.) } else { format!("{:.0} MB", f / 1_048_576.) }
    } else {
        format!("{:.1} GB", f / 1_073_741_824.)
    }
}


#[cfg(test)]
mod layout_regression_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn navigation_fits_tiled_windows(cx: &mut TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_, cx| crate::layout_test_launcher(cx));
        for width in [480., 600., 800., 1050., 1280.] {
            cx.simulate_resize(gpui::size(px(width), px(600.)));
            cx.refresh().unwrap();
            cx.run_until_parked();
            let mut previous_right = px(0.);
            for label in ["nav-Home", "nav-Instances", "nav-Mods", "nav-Servers", "nav-Skins", "nav-Create"] {
                let bounds = cx.debug_bounds(label).expect("navigation item rendered");
                assert!(bounds.left() >= previous_right, "navigation overlaps at {width}px: {label} {bounds:?}");
                assert!(bounds.right() <= px(width), "navigation outside {width}px: {label} {bounds:?}");
                previous_right = bounds.right();
            }
            let account = cx.debug_bounds("top-account").expect("account rendered");
            assert!(account.left() >= previous_right && account.right() <= px(width), "account outside navigation at {width}px: {account:?}");
        }
    }

    #[gpui::test]
    fn wrapped_confirmation_actions_stay_inside_card(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| crate::layout_test_launcher(cx));
        cx.simulate_resize(gpui::size(px(534.), px(288.)));
        cx.update(|window, app| view.update(app, |launcher, cx| {
            launcher.open_modal(
                "Delete e?",
                Some("The project and its textures are deleted from the launcher. Copies installed in instances or exported .zips stay. This can’t be undone."),
                vec![], "Delete", true, None, Rc::new(|_, _, _, _| {}), window, cx,
            );
        }));
        cx.refresh().unwrap();
        cx.run_until_parked();
        let card = cx.debug_bounds("modal-card").expect("modal card was rendered");
        let actions = cx.debug_bounds("modal-actions").expect("modal actions were rendered");
        assert!(actions.bottom() <= card.bottom() - px(8.), "actions {actions:?} overflow card {card:?}");
        assert!(actions.left() >= card.left() && actions.right() <= card.right(), "actions {actions:?} overflow card {card:?}");
    }
}
