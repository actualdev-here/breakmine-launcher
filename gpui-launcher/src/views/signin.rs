//! Sign in (src/js/views/signin.js): Breakmine OAuth, password login/register, or an offline profile.
use crate::app::Route;
use crate::pano::Pano;
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{account_api, assets, gpui_accounts_path, local_accounts, oauth, Launcher};
use gpui::{div, img, prelude::*, px, Context, Entity, Subscription, Window};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct State {
    mode: &'static str,
    busy: bool,
    error: String,
    note: String,
    show_password: bool,
    name: Entity<TextInput>,
    pass: Entity<TextInput>,
    pub pano: Entity<Pano>,
    cancel: Option<Arc<AtomicBool>>,
    focus_pass: bool,
    _subs: Vec<Subscription>,
}

impl State {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        let name = cx.new(|cx| TextInput::with_placeholder(cx, "steve"));
        let pass = cx.new(|cx| {
            let mut t = TextInput::with_placeholder(cx, "");
            t.set_masked(true);
            t
        });
        let subs = vec![
            cx.subscribe(&name, |this: &mut Launcher, _, ev: &TextEvent, cx| {
                if matches!(ev, TextEvent::Submit) {
                    if this.v.signin.mode == "offline" {
                        this.signin_offline(cx);
                    } else if this.v.signin.pass.read(cx).value().is_empty() {
                        // Enter in the name field moves on to the password (signin.js onKey).
                        this.v.signin.focus_pass = true;
                        cx.notify();
                    } else {
                        this.signin_password(cx);
                    }
                }
            }),
            cx.subscribe(&pass, |this: &mut Launcher, _, ev: &TextEvent, cx| {
                if matches!(ev, TextEvent::Submit) {
                    this.signin_password(cx);
                }
            }),
        ];
        Self { mode: "choose", busy: false, error: String::new(), note: String::new(), show_password: false, name, pass, pano: cx.new(|_| Pano::new()), cancel: None, focus_pass: false, _subs: subs }
    }

    /// Open the page with a name filled in and a note (an expired session).
    pub fn prefill(&mut self, name: &str, note: &str, cx: &mut Context<Launcher>) {
        let n = name.to_string();
        self.name.update(cx, |t, cx| {
            t.set_text(n);
            cx.notify();
        });
        self.note = note.to_string();
        self.mode = "password";
        self.error.clear();
    }
}

fn valid_username(name: &str) -> bool {
    (3..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Launcher {
    fn signin_name(&self, cx: &Context<Self>) -> String {
        self.v.signin.name.read(cx).value().trim().to_string()
    }

    fn check_signin(&self, need_password: bool, cx: &Context<Self>) -> String {
        if !valid_username(&self.signin_name(cx)) {
            return "Usernames are 3–32 letters, numbers, _ or -.".into();
        }
        let len = self.v.signin.pass.read(cx).value().len();
        if need_password && !(6..=128).contains(&len) {
            return "Passwords are 6–128 characters.".into();
        }
        String::new()
    }

    fn finish_signin(&mut self, cx: &mut Context<Self>) {
        let s = &mut self.v.signin;
        s.mode = "choose";
        s.note.clear();
        s.error.clear();
        s.show_password = false;
        s.pass.update(cx, |t, cx| {
            t.clear();
            cx.notify();
        });
        s.name.update(cx, |t, cx| {
            t.clear();
            cx.notify();
        });
        self.refresh_accounts();
        self.go(Route::Home, cx);
    }

    fn signin_password(&mut self, cx: &mut Context<Self>) {
        if self.v.signin.busy {
            return;
        }
        self.v.signin.error = self.check_signin(true, cx);
        if !self.v.signin.error.is_empty() {
            return cx.notify();
        }
        let name = self.signin_name(cx);
        let password = self.v.signin.pass.read(cx).value().to_string();
        let api = self.store.pointer("/settings/apiUrl").and_then(Value::as_str).unwrap_or("https://api.breakmine.com").to_string();
        self.v.signin.busy = true;
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let (n, a) = (name.clone(), api.clone());
            let result = executor
                .spawn(async move {
                    let login = account_api::login_or_register(&a, &n, &password)?;
                    local_accounts::upsert_oauth(&gpui_accounts_path(), &n, login.token, login.expires, &a).map(|_| login.created)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.v.signin.busy = false;
                match result {
                    Ok(created) => {
                        this.info(if created { format!("Created breakmine.com account {name}") } else { format!("Signed in as {name}") }, cx);
                        this.finish_signin(cx);
                    }
                    Err(e) => this.v.signin.error = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn signin_oauth(&mut self, cx: &mut Context<Self>) {
        if self.v.signin.busy {
            return;
        }
        self.v.signin.error.clear();
        self.v.signin.busy = true;
        let cancel = Arc::new(AtomicBool::new(false));
        self.v.signin.cancel = Some(cancel.clone());
        cx.notify();
        let api = self.store.pointer("/settings/apiUrl").and_then(Value::as_str).unwrap_or("https://api.breakmine.com").to_string();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let prepare_api = api.clone();
            let (attempt, url) = match executor.spawn(async move { oauth::prepare(&prepare_api) }).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = view.update(cx, |this, cx| {
                        this.v.signin.busy = false;
                        this.v.signin.error = e;
                        cx.notify();
                    });
                    return;
                }
            };
            let opened = view
                .update(cx, |_, cx| {
                    if cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    cx.open_url(&url);
                    true
                })
                .unwrap_or(false);
            if !opened {
                let _ = view.update(cx, |this, cx| {
                    this.v.signin.busy = false;
                    cx.notify();
                });
                return;
            }
            let result = executor
                .spawn(async move { oauth::finish(attempt, cancel, |cr| local_accounts::upsert_oauth(&gpui_accounts_path(), &cr.name, cr.token, cr.expires, &api).map(|_| ())) })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.v.signin.busy = false;
                this.v.signin.cancel = None;
                match result {
                    Ok(()) => {
                        this.info("Breakmine profile connected", cx);
                        this.finish_signin(cx);
                    }
                    Err(e) => this.v.signin.error = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn signin_offline(&mut self, cx: &mut Context<Self>) {
        self.v.signin.error = self.check_signin(false, cx);
        if !self.v.signin.error.is_empty() {
            return cx.notify();
        }
        let name = self.signin_name(cx);
        match local_accounts::add_offline(&gpui_accounts_path(), &name) {
            Ok(prefs) => {
                // add_offline selects the new (or existing) profile.
                self.account_prefs = prefs;
                self.finish_signin(cx);
            }
            Err(e) => {
                self.v.signin.error = e;
                cx.notify();
            }
        }
    }

    /// The account menu's "Sign out": the account in use.
    pub fn sign_out(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let id = self.selected_account_id();
        if !id.is_empty() {
            self.remove_account(&id, cx);
        }
        if self.accounts().is_empty() {
            self.go(Route::Signin, cx);
        }
    }

    pub fn signin_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let set = self.store.pointer("/settings/panorama").and_then(Value::as_str).unwrap_or("day").to_string();
        let reduce = self.store.pointer("/settings/reduceMotion").and_then(Value::as_bool).unwrap_or(false);
        let pano = self.v.signin.pano.clone();
        pano.update(cx, |p, _| {
            p.set(&set);
            p.reduce_motion = reduce;
        });
        if std::mem::take(&mut self.v.signin.focus_pass) {
            self.v.signin.pass.read(cx).focus_handle_ref().focus(window);
        }
        let back = !self.accounts().is_empty();
        let s = &self.v.signin;
        let busy = s.busy;
        let mode = s.mode;
        let note = if !s.note.is_empty() {
            s.note.clone()
        } else {
            match mode {
                "offline" => "Choose a name for your offline profile.",
                "password" => "Sign in or create a Breakmine account.",
                _ => "Choose how you want to play.",
            }
            .to_string()
        };
        let mut form = div().w_full().flex().flex_col().gap(px(10.));
        if mode == "choose" {
            form = form
                .child(
                    Btn::new("si-oauth", if busy { "Waiting for browser…" } else { "Connect with Breakmine" })
                        .primary()
                        .tall()
                        .full()
                        .disabled(busy)
                        .leading(img(assets::path("assets/logo.png")).size(px(20.)))
                        .on_click(cx.listener(|this, _, _, cx| this.signin_oauth(cx))),
                )
                .child(Btn::new("si-pw", "Sign in with password").tall().full().disabled(busy).on_click(cx.listener(|this, _, window, cx| {
                    this.v.signin.mode = "password";
                    this.v.signin.error.clear();
                    this.v.signin.name.read(cx).focus_handle_ref().focus(window);
                    cx.notify();
                })))
                .child(Btn::new("si-off", "Play offline").tall().full().disabled(busy).on_click(cx.listener(|this, _, window, cx| {
                    this.v.signin.mode = "offline";
                    this.v.signin.error.clear();
                    this.v.signin.name.read(cx).focus_handle_ref().focus(window);
                    cx.notify();
                })));
            if busy {
                form = form.child(Btn::new("si-stop", "Cancel").full().on_click(cx.listener(|this, _, _, cx| {
                    if let Some(c) = this.v.signin.cancel.take() {
                        c.store(true, Ordering::Relaxed);
                    }
                    this.v.signin.busy = false;
                    cx.notify();
                })));
            }
        } else {
            let tall_box = |input: &Entity<TextInput>, window: &Window, cx: &gpui::App| input_box(input, window, cx, false).h(px(46.)).rounded(px(12.)).px(px(14.)).border_color(c(if input.read(cx).focus_handle_ref().is_focused(window) { 0x3a3a3a } else { LINE2 }));
            form = form.child(field("Username", tall_box(&self.v.signin.name, window, cx)));
            if mode == "password" {
                let show = self.v.signin.show_password;
                let pass_box = tall_box(&self.v.signin.pass, window, cx).pr(px(50.)).relative().child(
                    div().absolute().right(px(5.)).top(px(2.)).child(icon_btn("si-eye", "eye", false, show).on_click(cx.listener(|this, _, _, cx| {
                        this.v.signin.show_password = !this.v.signin.show_password;
                        let masked = !this.v.signin.show_password;
                        this.v.signin.pass.update(cx, |t, cx| {
                            t.set_masked(masked);
                            cx.notify();
                        });
                        cx.notify();
                    }))),
                );
                form = form.child(field("Password", pass_box));
            }
            let primary = Btn::new("si-go", if busy { "Signing in…" } else if mode == "password" { "Login or register" } else { "Create offline profile" })
                .primary()
                .tall()
                .full()
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| if mode == "password" { this.signin_password(cx) } else { this.signin_offline(cx) }));
            form = form.child(primary).child(Btn::new("si-back", "Back").full().disabled(busy).on_click(cx.listener(|this, _, _, cx| {
                let s = &mut this.v.signin;
                s.mode = "choose";
                s.error.clear();
                s.show_password = false;
                s.pass.update(cx, |t, cx| {
                    t.clear();
                    t.set_masked(true);
                    cx.notify();
                });
                cx.notify();
            })));
        }
        if !self.v.signin.error.is_empty() {
            form = form.child(div().px(px(12.)).py(px(10.)).border_1().border_color(c(0x4a1d1d)).rounded(px(10.)).bg(c(0x160808)).text_color(c(DANGER)).text_size(px(13.)).line_height(gpui::relative(1.45)).child(self.v.signin.error.clone()));
        }
        if back {
            form = form.child(div().flex().justify_center().child(link("si-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                if let Some(c) = this.v.signin.cancel.take() {
                    c.store(true, Ordering::Relaxed);
                }
                let s = &mut this.v.signin;
                s.busy = false;
                s.mode = "choose";
                s.error.clear();
                s.note.clear();
                this.go(Route::Home, cx);
            }))));
        }
        let card = div()
            .w_full()
            .max_w(px(420.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .pt(px(40.))
            .px(px(36.))
            .pb(px(32.))
            .border_1()
            .border_color(c(0x1f1f1f))
            .rounded(px(22.))
            .bg(c(0x000000))
            .child(img(assets::path("assets/game/launcher_logo.png")).w_full().max_w(px(250.)).h(px(67.2)).object_fit(gpui::ObjectFit::Contain))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(pixel_text(if back { "Add an account" } else { "Sign in to play" }, 24.))
                    .child(div().text_size(px(13.)).text_color(c(DIM)).line_height(gpui::relative(1.5)).text_center().child(note)),
            )
            .child(form)
            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).text_center().child("Offline profiles can play singleplayer. Multiplayer and skins need a breakmine.com account."))
            .child(
                div()
                    .max_w(px(310.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(img(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mojang-mark.svg")).size(px(22.)))
                            .child(img(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/microsoft-mark.svg")).size(px(14.))),
                    )
                    .child(div().text_size(px(10.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).text_center().child("Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.")),
            );
        div()
            .relative()
            .size_full()
            .flex_1()
            .child(div().absolute().inset_0().opacity(0.6).child(pano))
            .child(div().relative().size_full().min_h(px(600.)).flex().items_center().justify_center().pt(px(24.)).px(px(16.)).pb(px(48.)).child(card))
    }
}
