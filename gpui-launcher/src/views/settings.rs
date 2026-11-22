//! Settings (src/js/views/settings.js): general, game window, game build, appearance, accounts, about.
use crate::app::Route;
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, local_accounts, offline_download, util, Launcher};
use gpui::{div, img, prelude::*, px, AnyElement, Context, Entity, FontWeight, SharedString, Subscription, Window};
use serde_json::{json, Value};

pub struct State {
    section: &'static str,
    licenses: Option<String>,
    advanced: bool,
    latest: Option<String>,
    checking: bool,
    inputs: Vec<(&'static str, Entity<TextInput>)>,
    _subs: Vec<Subscription>,
}

impl State {
    pub fn reset_latest(&mut self) {
        self.latest = None;
    }
}

impl Default for State {
    fn default() -> Self {
        Self { section: "general", licenses: None, advanced: false, latest: None, checking: false, inputs: Vec::new(), _subs: Vec::new() }
    }
}

const MENU: [(&str, &str, &str); 6] = [
    ("general", "General", "sliders"),
    ("game", "Game window", "monitor"),
    ("build", "Game build", "package"),
    ("look", "Appearance", "paint"),
    ("accounts", "Accounts", "user"),
    ("about", "About", "info"),
];

/// Text settings: key, placeholder.
const FIELDS: [(&str, &str); 8] = [
    ("windowWidth", ""),
    ("windowHeight", ""),
    ("onlineUrl", "https://game.breakmine.com/"),
    ("localUrl", "http://localhost:8000/"),
    ("customUrl", "https://play.example.com/"),
    ("apiUrl", "https://api.breakmine.com"),
    ("catalogUrl", "https://mods.breakmine.com/api/mods"),
    ("offlineSource", "https://datastore.breakmine.com/"),
];

fn valid_url(v: &str) -> bool {
    let l = v.to_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && !v.contains(char::is_whitespace) && v.len() > 8
}

/// `.set-row`.
fn set_row(label: impl Into<SharedString>, sub: Option<AnyElement>, control: impl IntoElement, first: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(16.))
        .py(px(14.))
        .when(!first, |d| d.border_t_1().border_color(c(0x121212)))
        .child(div().flex().flex_col().gap(px(3.)).flex_1().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(label.into())).children(sub))
        .child(control)
}

fn sub_text(t: impl Into<SharedString>) -> AnyElement {
    div().text_size(px(12.)).text_color(c(DIM)).child(t.into()).into_any_element()
}

fn set_card() -> gpui::Div {
    div().flex().flex_col().border_1().border_color(c(LINE)).rounded(px(18.)).px(px(20.)).py(px(8.)).bg(c(CARD))
}

impl Launcher {
    fn setting_bool(&self, key: &str) -> bool {
        self.store.pointer(&format!("/settings/{key}")).and_then(Value::as_bool).unwrap_or(false)
    }
    fn setting_str(&self, key: &str) -> String {
        match self.store.pointer(&format!("/settings/{key}")) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    }
    pub fn set_setting(&mut self, key: &str, v: Value) {
        if !self.store["settings"].is_object() {
            self.store["settings"] = json!({});
        }
        self.store["settings"][key] = v;
        self.save_store();
    }

    pub fn settings_mount(&mut self, cx: &mut Context<Self>) {
        if self.v.settings.inputs.is_empty() {
            let mut subs = Vec::new();
            for (key, ph) in FIELDS {
                let value = self.setting_str(key);
                let input = cx.new(|cx| {
                    let mut t = TextInput::with_placeholder(cx, ph);
                    t.set_text(value);
                    t
                });
                subs.push(cx.subscribe(&input, move |this: &mut Launcher, input, ev: &TextEvent, cx| {
                    let v = input.read(cx).value().trim().to_string();
                    match key {
                        "windowWidth" | "windowHeight" => {
                            let (lo, hi, def) = if key == "windowWidth" { (640, 7680, 1280) } else { (400, 4320, 720) };
                            if let Ok(n) = v.parse::<u32>() {
                                if (lo..=hi).contains(&n) {
                                    this.set_setting(key, json!(n));
                                }
                            } else if matches!(ev, TextEvent::Submit) {
                                this.set_setting(key, json!(def));
                            }
                        }
                        _ => {
                            if (v.is_empty() && key == "customUrl") || valid_url(&v) {
                                this.set_setting(key, json!(v));
                                if key == "offlineSource" && matches!(ev, TextEvent::Submit) {
                                    this.v.settings.latest = None;
                                    this.check_offline(cx);
                                }
                            } else if matches!(ev, TextEvent::Submit) {
                                this.error("Enter a full URL starting with http:// or https://", cx);
                            }
                        }
                    }
                    cx.notify();
                }));
                self.v.settings.inputs.push((key, input));
            }
            self.v.settings._subs = subs;
        }
        self.check_offline(cx);
    }

    fn input_for(&self, key: &str) -> Entity<TextInput> {
        self.v.settings.inputs.iter().find(|(k, _)| *k == key).map(|(_, i)| i.clone()).expect("settings input")
    }

    /// The newest offline version, for the Home update notice (only when an instance uses it).
    pub fn check_offline_quiet(&mut self, cx: &mut Context<Self>) {
        let uses_newest = self.instances.iter().any(|i| self.build_of(&i.inst) == "offline" && i.inst.game_version.is_empty());
        if uses_newest {
            self.check_offline(cx);
        }
    }

    pub fn latest_offline(&self) -> Option<String> {
        self.v.settings.latest.clone()
    }

    fn check_offline(&mut self, cx: &mut Context<Self>) {
        if self.v.settings.checking || self.v.settings.latest.is_some() {
            return;
        }
        self.v.settings.checking = true;
        let source = self.setting_str("offlineSource");
        let source = if source.is_empty() { "https://datastore.breakmine.com/".to_string() } else { source };
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let latest = executor.spawn(async move { offline_download::latest(&source).ok() }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.settings.checking = false;
                this.v.settings.latest = latest;
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_row(&mut self, key: &'static str, label: &'static str, hint: &'static str, first: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.setting_bool(key);
        set_row(
            label,
            if hint.is_empty() { None } else { Some(sub_text(hint)) },
            switch(SharedString::from(format!("sw-{key}")), on).on_click(cx.listener(move |this, _, _, cx| {
                this.set_setting(key, json!(!on));
                if key == "autostart" {
                    if let Err(e) = set_autostart(!on) {
                        this.set_setting(key, json!(on));
                        this.error(e, cx);
                    }
                }
                cx.notify();
            })),
            first,
        )
    }

    fn general_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let dir = self.game_dir.clone();
        let discord = !self.setting_str("discordClientId").is_empty();
        set_card()
            .child(set_row(
                "Game folder",
                Some(mono(format!("{dir}/instances")).text_color(c(DIM)).into_any_element()),
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Btn::new("open-dir", "Open").sm().icon("folder").on_click(cx.listener(|this, _, _, cx| cx.open_with_system(std::path::Path::new(&this.game_dir)))))
                    .child(Btn::new("game-dir", "Change").sm().on_click(cx.listener(|this, _, _, cx| this.pick_game_dir(cx)))),
                true,
            ))
            .child(self.toggle_row("autostart", "Open Breakmine Launcher when your computer starts", "Starts minimized", false, cx))
            .child(self.toggle_row("notifications", "Session summaries", "Adds “Played … for …” to the inbox after each session", false, cx))
            .child(if discord {
                self.toggle_row("discordPresence", "Discord Rich Presence", "Show your instance and play time while the game is open. Requires Discord desktop.", false, cx).into_any_element()
            } else {
                set_row("Discord Rich Presence", Some(sub_text("Discord integration is not configured yet.")), div().text_size(px(12.)).text_color(c(DIM)).child("Coming soon"), false).into_any_element()
            })
            .into_any_element()
    }

    fn game_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mini = |label: &'static str, input: Entity<TextInput>, window: &Window, cx: &App| {
            div().flex().flex_col().gap(px(4.)).text_size(px(11.)).text_color(c(DIM)).child(label).child(div().w(px(96.)).child(input_box(&input, window, cx, true).h(px(38.))))
        };
        let on_launch = self.setting_str("onLaunch");
        let mut s = seg();
        for (id, label) in [("open", "Keep open"), ("minimize", "Minimize"), ("close", "Hide")] {
            s = s.child(pill(SharedString::from(format!("ol-{id}")), label, on_launch == id, false).on_click(cx.listener(move |this, _, _, cx| {
                this.set_setting("onLaunch", json!(id));
                cx.notify();
            })));
        }
        let hint = match on_launch.as_str() {
            "close" => "The launcher hides and quits when the game closes",
            "minimize" => "Comes back when the game closes",
            _ => "The launcher stays where it is",
        };
        set_card()
            .child(set_row(
                "Window size",
                None,
                div()
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .child(mini("Width", self.input_for("windowWidth"), window, cx))
                    .child(div().pb(px(12.)).child(icon("times", 16., c(DIM2))))
                    .child(mini("Height", self.input_for("windowHeight"), window, cx)),
                true,
            ))
            .child(self.toggle_row("fullscreen", "Start in fullscreen", "", false, cx))
            .child(set_row("While you play", Some(sub_text(hint)), s, false))
            .into_any_element()
    }

    fn offline_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let installed = offline_download::current(&crate::launcher_data()).map(|(v, _)| v);
        let latest = self.v.settings.latest.clone();
        let downloading = self.offline_task.is_some();
        let update = matches!((&installed, &latest), (Some(i), Some(l)) if i != l);
        let progress = self.offline_task.as_ref().and_then(|t| t.progress.lock().ok().map(|p| (p.received, p.total, p.phase)));
        let status = if downloading {
            format!("Downloading… {}", progress.map(|(r, t, ph)| match t { Some(t) if t > 0 => format!("{}%", r * 100 / t), _ => ph.to_string() }).unwrap_or_default())
        } else if let Some(i) = &installed {
            format!("Installed: {i}{}", if update { format!(" · {} available", latest.clone().unwrap_or_default()) } else if latest.is_some() { " · up to date".into() } else { String::new() })
        } else if let Some(l) = &latest {
            format!("Not downloaded · latest is {l}")
        } else {
            "Not downloaded".into()
        };
        let label = if installed.is_some() { if update { "Update" } else { "Re-download" } } else { "Download" };
        let btn = Btn::new("offline-dl", label).sm().icon(if installed.is_some() { "rotate" } else { "down" }).disabled(downloading);
        let btn = if update || installed.is_none() { btn.primary() } else { btn };
        let mut col = card()
            .child(div().flex().items_center().gap(px(10.)).child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Offline build")).child(btn.on_click(cx.listener(|this, _, _, cx| {
                this.start_offline_download(cx);
                this.v.settings.latest = None;
            }))))
            .child(mono(status).text_color(c(DIM)));
        if downloading {
            let pct = progress.and_then(|(r, t, _)| t.filter(|t| *t > 0).map(|t| (r * 100 / t) as usize));
            let mut blocks = div().flex().gap(px(2.));
            for i in 0..20 {
                blocks = blocks.child(div().flex_1().h(px(6.)).bg(c(if pct.is_some_and(|p| i < (p + 2) / 5) { ACCENT } else { LINE2 })));
            }
            col = col.child(blocks);
        }
        col.child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child("The single-file build with music, from datastore.breakmine.com. Plays without internet; it updates itself at launch when you’re online.")).into_any_element()
    }

    fn build_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let build = self.setting_str("build");
        let build = if build.is_empty() { "online".to_string() } else { build };
        let dev = self.setting_bool("showDevBuilds");
        let mut opts = vec![DdOpt::new("online", "Online").sub("game.breakmine.com · always the newest"), DdOpt::new("offline", "Offline").sub("Downloaded single-file build")];
        if dev || build == "local" {
            opts.push(DdOpt::new("local", "Local dev server").sub("npm run devs:http"));
        }
        if dev || build == "custom" {
            opts.push(DdOpt::new("custom", "Custom URL").sub("Your own Breakmine host"));
        }
        let dd_id: SharedString = "set-build".into();
        let open = self.dd_open.as_ref() == Some(&dd_id);
        let tid = dd_id.clone();
        let dd = Dropdown::new(dd_id, opts, build.clone(), open)
            .width(300.)
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
            .on_pick(cx.listener(|this, v: &SharedString, _, cx| {
                this.set_setting("build", json!(v.to_string()));
                this.dd_open = None;
                cx.notify();
            }));
        let inputs = self.v.settings.inputs.clone();
        let url_field = move |key: &'static str, label: &'static str, window: &Window, cx: &App| {
            let input = inputs.iter().find(|(k, _)| *k == key).map(|(_, i)| i.clone()).expect("settings input");
            field(label, input_box(&input, window, cx, true)).flex_1()
        };
        let source: Option<AnyElement> = match build.as_str() {
            "local" => Some(url_field("localUrl", "Dev server URL", window, cx).into_any_element()),
            "custom" => Some(url_field("customUrl", "Game URL", window, cx).into_any_element()),
            "online" => Some(url_field("onlineUrl", "Game URL", window, cx).into_any_element()),
            _ => None,
        };
        let advanced = self.v.settings.advanced;
        let mut card = set_card()
            .p(px(20.))
            .gap(px(20.))
            .child(div().flex().flex_wrap().items_start().gap(px(14.)).child(div().flex_1().min_w(px(240.)).child(field("Default build for instances", dd))).children(source.map(|s| div().flex_1().min_w(px(240.)).flex().flex_col().child(s))))
            .child(self.offline_card(cx))
            .child(self.toggle_row("showDevBuilds", "Show dev builds", "Local dev server and custom URL options", false, cx))
            .child(
                div()
                    .border_t_1()
                    .border_color(c(0x121212))
                    .pt(px(14.))
                    .child(
                        div()
                            .id("advanced")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(0xd4d4d4))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.v.settings.advanced = !this.v.settings.advanced;
                                cx.notify();
                            }))
                            .child(icon(if advanced { "chevron" } else { "chevron-right" }, 16., c(0xd4d4d4)))
                            .child("Advanced"),
                    ),
            );
        if advanced {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(url_field("apiUrl", "Account API", window, cx))
                    .child(url_field("catalogUrl", "Mod catalog", window, cx))
                    .child(url_field("offlineSource", "Offline build source", window, cx)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(card)
            .child(div().flex().flex_wrap().text_size(px(12.)).text_color(c(DIM2)).child("Every launch adds ").child(mono("?launcher=1")).child(" so the game signs in with your launcher account and loads the instance’s mods."))
            .into_any_element()
    }

    fn look_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let art = self.setting_str("homeArt");
        let show_mascot = !self.setting_bool("hideHomeMascot");
        let art = if art.is_empty() { "classic".to_string() } else { art };
        let mut choices = div().flex().flex_wrap().gap(px(12.));
        for (id, label, src) in [("classic", "Classic", "assets/game/beta_art.webp"), ("sky", "Sky", "assets/game/hoof_sky.png"), ("launcher", "Day & Night", "assets/game/launcher_art.png")] {
            let on = art == id;
            choices = choices.child(
                div()
                    .id(SharedString::from(format!("art-{id}")))
                    .w(px(190.))
                    .p(px(5.))
                    .border_1()
                    .border_color(c(if on { ACCENT } else { LINE2 }))
                    .rounded(px(10.))
                    .bg(c(CARD))
                    .text_color(c(if on { ACCENT } else { MUTED }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_setting("homeArt", json!(id));
                        cx.notify();
                    }))
                    .child(img(assets::path(src)).w_full().h(px(90.)).rounded(px(6.)).object_fit(gpui::ObjectFit::Cover))
                    .child(div().pt(px(8.)).px(px(5.)).pb(px(4.)).text_size(px(12.)).child(label)),
            );
        }
        set_card()
            .p(px(20.))
            .gap(px(22.))
            .child(div().flex().flex_col().gap(px(12.)).child(div().flex().flex_col().gap(px(3.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Home artwork")).child(sub_text("Choose the picture on the Home page · art by Hoof"))).child(choices))
            .child(set_row("Show home mascot", Some(sub_text("Show the little miner on your Home artwork")),
                switch("sw-home-mascot", show_mascot).on_click(cx.listener(move |this, _, _, cx| {
                    this.set_setting("hideHomeMascot", json!(show_mascot));
                    cx.notify();
                })), false))
            .child(self.toggle_row("reduceMotion", "Reduce motion", "Turn off hover and launch animations", false, cx))
            .into_any_element()
    }

    fn accounts_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.selected_account_id();
        let mut card = set_card().p(px(20.)).gap(px(14.));
        let now = util::now();
        for (id, name, offline, expired, local) in self.accounts() {
            let expires = if local {
                self.account_prefs.online_profiles.iter().find(|p| p.id == id).map(|p| p.expires).unwrap_or(0)
            } else {
                self.store["accounts"].as_array().into_iter().flatten().find(|a| a["id"].as_str() == Some(&id)).and_then(|a| a["expires"].as_u64()).unwrap_or(0)
            };
            let days = if expires > now { ((expires - now) as f64 / 86400.).round() as u64 } else { 0 };
            let sub = if offline {
                "Offline profile · singleplayer only".to_string()
            } else if expired {
                "breakmine.com · session expired".into()
            } else if local {
                "breakmine.com · signed in with OAuth".into()
            } else {
                format!("breakmine.com · signed in for {days} more day{}", if days == 1 { "" } else { "s" })
            };
            let av = self.avatar(Some((id.clone(), name.clone(), offline)), 40., cx);
            let (use_id, out_id, re_name) = (id.clone(), id.clone(), name.clone());
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(av)
                    .child(div().flex().flex_col().gap(px(3.)).mr_auto().child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(name.clone())).child(div().text_size(px(13.)).text_color(c(DIM)).child(sub)))
                    .when(expired, |d| {
                        d.child(Btn::new(SharedString::from(format!("re-{id}")), "Sign in again").primary().sm().on_click(cx.listener(move |this, _, _, cx| {
                            this.v.signin.prefill(&re_name, "Your breakmine.com session expired. Sign in again.", cx);
                            this.go(Route::Signin, cx);
                        })))
                    })
                    .child(if id == active {
                        div().flex().items_center().h(px(26.)).px(px(10.)).rounded(px(6.)).border_1().border_color(c(LINE2)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(ACCENT)).child("ACTIVE").into_any_element()
                    } else {
                        Btn::new(SharedString::from(format!("use-{id}")), "Use").sm().on_click(cx.listener(move |this, _, _, cx| this.pick_account(&use_id, cx))).into_any_element()
                    })
                    .child(Btn::new(SharedString::from(format!("out-{id}")), "Sign out").sm().on_click(cx.listener(move |this, _, _, cx| this.remove_account(&out_id, cx)))),
            );
        }
        card = card.child(
            div().child(
                div()
                    .id("add-acct")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(12.))
                    .rounded(px(9.))
                    .border_1()
                    .border_dashed()
                    .border_color(c(0x2a2a2a))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(c(0xd4d4d4))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x111111)))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Route::Signin, cx)))
                    .child(icon("plus", 16., c(0xd4d4d4)))
                    .child("Add account"),
            ),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(card)
            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child("Passwords are never stored. The launcher keeps the 7-day session token from breakmine.com and hands it to the game."))
            .into_any_element()
    }

    fn about_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let ver = offline_download::current(&crate::launcher_data()).map(|(v, _)| v);
        let licenses = self.v.settings.licenses.clone();
        let link_btn = |id: &'static str, label: &'static str, url: &'static str| Btn::new(id, label).icon("external").icon_size(12.).on_click(move |_, _, cx| cx.open_url(url));
        set_card()
            .p(px(28.))
            .gap(px(20.))
            .items_start()
            .child(img(assets::path("assets/game/launcher_logo.png")).w_full().max_w(px(320.)).h(px(86.)).object_fit(gpui::ObjectFit::Contain))
            .child(div().flex().flex_col().gap(px(4.)).child(mono(format!("Launcher 0.1.0{}", ver.map(|v| format!(" · Game {v}")).unwrap_or_default())).text_size(px(13.))).child(div().text_size(px(13.)).text_color(c(DIM)).child(format!("Data: {}", self.game_dir))))
            .child(div().flex().items_center().gap(px(6.)).text_size(px(13.)).child(div().text_color(c(DIM)).child("Launcher by")).child(link("by", "ActualDev").child(icon("external", 10., c(0xd4d4d4))).on_click(|_, _, cx| cx.open_url("https://actualdev.vercel.app/"))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(14.))
                    .border_1()
                    .border_color(c(LINE2))
                    .rounded(px(12.))
                    .bg(c(CARD))
                    .child(img(assets::path("assets/game/patchwork_wordmark.png")).h(px(28.)).w(px(200.)).object_fit(gpui::ObjectFit::Contain))
                    .child(div().text_size(px(12.)).text_color(c(DIM)).child("Mods run on Patchwork, Breakmine’s mod loader.")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(link_btn("src", "Source code", "https://github.com/Breakmine-Team/breakmine-revived"))
                    .child(link_btn("site", "Mods site", "https://mods.breakmine.com/"))
                    .child(Btn::new("lic", if licenses.is_some() { "Hide licenses" } else { "Licenses" }).icon("info").icon_size(12.).on_click(cx.listener(|this, _, _, cx| {
                        this.v.settings.licenses = match this.v.settings.licenses {
                            Some(_) => None,
                            None => Some(std::fs::read_to_string(assets::path("assets/LICENSES.txt")).unwrap_or_default()),
                        };
                        cx.notify();
                    }))),
            )
            .children(licenses.map(|l| {
                div()
                    .id("licenses")
                    .max_h(px(260.))
                    .w_full()
                    .overflow_y_scroll()
                    .p(px(14.))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(10.))
                    .bg(c(0x000000))
                    .text_color(c(MUTED))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .line_height(gpui::relative(1.5))
                    .child(l)
            }))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(c(DIM2))
                    .line_height(gpui::relative(1.5))
                    .child("Breakmine artwork (Home art by Hoof), panoramas, logos and the default skin © SpinningCubes & Breakmine contributors, used under CC BY-NC 4.0.")
                    .child("Not an official Minecraft product. Not approved by or associated with Mojang."),
            )
            .into_any_element()
    }

    pub fn settings_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.v.settings.inputs.is_empty() {
            self.settings_mount(cx);
        }
        let compact = f32::from(window.viewport_size().width) <= 760.;
        let section = self.v.settings.section;
        let label = MENU.iter().find(|m| m.0 == section).map(|m| m.1).unwrap_or("General");
        let mut menu = div()
            .w(px(if f32::from(window.viewport_size().width) <= 1100. { 200. } else { 240. }))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(4.))
            .py(px(22.))
            .px(px(12.))
            .border_r_1()
            .border_color(c(0x111111))
            .when(compact, |d| d.w_full().flex_row().flex_wrap().py(px(12.)).border_r_0())
            .when(!compact, |d| d.child(pixel_text("Settings", 32.).ml(px(10.)).mb(px(18.))));
        for (id, l, ic) in MENU {
            let on = section == id;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("set-{id}")))
                    .h(px(42.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if on { c(LINE2) } else { ca(0x00000000) })
                    .when(on, |d| d.bg(c(0x111111)))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(c(if on { 0xfafafa } else { MUTED }))
                    .cursor_pointer()
                    .hover(|s| s.text_color(c(0xfafafa)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.settings.section = id;
                        if id == "build" || id == "about" {
                            this.check_offline(cx);
                        }
                        cx.notify();
                    }))
                    .child(icon(ic, 16., c(if on { 0xfafafa } else { MUTED })))
                    .child(l),
            );
        }
        let body = match section {
            "game" => self.game_section(window, cx),
            "build" => self.build_section(window, cx),
            "look" => self.look_section(cx),
            "accounts" => self.accounts_section(cx),
            "about" => self.about_section(cx),
            _ => self.general_section(cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .when(compact, |d| d.flex_col())
            .child(menu)
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(20.)).pt(px(22.)).px(px(if compact { 16. } else { 28. })).pb(px(28.)).child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child(label)).child(body))
    }

    fn pick_game_dir(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose game folder".into()) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(p) = paths.into_iter().next() else { return };
            let _ = view.update(cx, |this, cx| {
                let dir = p.to_string_lossy().into_owned();
                this.set_setting("gameDir", json!(dir.clone()));
                this.game_dir = dir;
                this.refresh();
                this.info("Game folder changed", cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Sign out of a locally managed account from Settings or the account menu.
    pub fn remove_account(&mut self, id: &str, cx: &mut Context<Self>) {
        let path = crate::gpui_accounts_path();
        if self.account_prefs.online_profiles.iter().any(|p| p.id == id) {
            match local_accounts::remove_online(&path, id) {
                Ok((prefs, removed)) => {
                    self.account_prefs = prefs;
                    let api = if removed.api.is_empty() { self.setting_str("apiUrl") } else { removed.api.clone() };
                    let token = removed.token.clone();
                    cx.background_executor().spawn(async move { let _ = crate::oauth::revoke(&api, &token); }).detach();
                    self.info(format!("Signed out {}", removed.name), cx);
                }
                Err(e) => self.error(e, cx),
            }
        } else if self.account_prefs.offline_profiles.iter().any(|p| p.id == id) {
            match local_accounts::remove_offline(&path, id) {
                Ok(prefs) => self.account_prefs = prefs,
                Err(e) => self.error(e, cx),
            }
        } else {
            self.error("This legacy account is read-only in Breakmine Launcher.", cx);
        }
        cx.notify();
    }
}

/// Manage the Linux autostart entry.
fn set_autostart(on: bool) -> Result<(), String> {
    let dir = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")).join("autostart");
    let file = dir.join("breakmine-launcher-gpui.desktop");
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=Breakmine Launcher\nExec=\"{}\" --minimized\nX-GNOME-Autostart-enabled=true\n", exe.display())).map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&file) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

use gpui::App;
