//! Skins (src/js/views/skins.js): 3D preview, skin library, uploads to breakmine.com and capes.
use crate::app::{FieldSpec, Route};
use crate::skin3d::SkinViewer;
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{account_api, assets, util, Launcher};
use base64::Engine;
use gpui::{div, img, prelude::*, px, AnimationExt, AnyElement, Context, Entity, FontWeight, SharedString, Window};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::rc::Rc;

// Match .skin-grid's repeat(auto-fill, minmax(120px, 1fr)).
fn library_width(viewport_width: f32) -> f32 {
    let available = (viewport_width - 56.).max(120.);
    if available >= 824. { available - 24. - 340. } else { available }
}
fn grid_card_width(available: f32, minimum: f32, gap: f32) -> f32 {
    let columns = ((available + gap) / (minimum + gap)).floor().max(1.);
    ((available - gap * (columns - 1.)) / columns).floor().max(minimum)
}
fn skin_card_width(viewport_width: f32) -> f32 {
    grid_card_width(library_width(viewport_width), 120., 12.)
}

pub struct State {
    sel: Option<String>,
    busy: bool,
    /// (capes, choice) from breakmine.com; None until loaded. choice: None = default cape, Some("") = none.
    capes: Option<(Vec<Cape>, Option<String>)>,
    cape_err: String,
    cape_busy: Option<String>,
    cape_for: Option<String>,
    pub viewer: Entity<SkinViewer>,
    pub home_viewer: Entity<SkinViewer>,
}

#[derive(Clone)]
struct Cape {
    id: String,
    name: String,
    png: Vec<u8>,
}

impl State {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        Self { sel: None, busy: false, capes: None, cape_err: String::new(), cape_busy: None, cape_for: None, viewer: cx.new(|_| SkinViewer::new(false)), home_viewer: cx.new(|_| SkinViewer::new(true)) }
    }
}

/// One entry of the library: the account's breakmine.com skin, the built-in one, or a launcher skin.
#[derive(Clone)]
struct Entry {
    id: String,
    name: String,
    slim: bool,
    builtin: bool,
    remote: bool,
    png: Vec<u8>,
}

fn data_png(src: &str) -> Option<Vec<u8>> {
    let b64 = src.strip_prefix("data:image/png;base64,")?;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

impl Launcher {
    /// (api, token) for the breakmine.com account in use (lib.rs `online_token`).
    pub fn online_token(&self) -> Result<(String, String, String), String> {
        let (id, name, offline) = self.active_account().ok_or("Capes need a breakmine.com account.")?;
        if offline {
            return Err("Capes need a breakmine.com account.".into());
        }
        let api = self.store.pointer("/settings/apiUrl").and_then(Value::as_str).unwrap_or("https://api.breakmine.com").to_string();
        if let Some(p) = self.account_prefs.online_profiles.iter().find(|p| p.id == id) {
            if p.token.is_empty() || p.expires <= util::now() {
                return Err(format!("Your breakmine.com session for {name} expired. Sign in again."));
            }
            return Ok((if p.api.is_empty() { api } else { p.api.clone() }, p.token.clone(), name));
        }
        let a = self.store["accounts"].as_array().into_iter().flatten().find(|a| a["id"].as_str() == Some(&id)).ok_or("Account not found.")?;
        let token = a["token"].as_str().unwrap_or("");
        if token.is_empty() || a["expires"].as_u64().unwrap_or(0) <= util::now() {
            return Err(format!("Your breakmine.com session for {name} expired. Sign in again."));
        }
        Ok((api, token.to_string(), name))
    }

    fn skin_entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        if let Some((_, name, false)) = self.active_account() {
            if let Some(Some(png)) = self.remote_skins.get(&name) {
                out.push(Entry { id: "remote".into(), name: "On breakmine.com".into(), slim: false, builtin: false, remote: true, png: png.clone() });
            }
        }
        out.push(Entry { id: "default".into(), name: "Breakmine default".into(), slim: false, builtin: true, remote: false, png: assets::default_skin() });
        let mut skins: Vec<Value> = self.store["skins"].as_array().cloned().unwrap_or_default();
        skins.sort_by(|a, b| b["lastUsed"].as_u64().unwrap_or(0).cmp(&a["lastUsed"].as_u64().unwrap_or(0)));
        for s in skins {
            let (Some(id), Some(file)) = (s["id"].as_str(), s["file"].as_str()) else { continue };
            let Ok(file) = util::safe_name(file) else { continue };
            let png = assets::read_cached(&crate::launcher_data().join("skins").join(file)).unwrap_or_default();
            out.push(Entry { id: id.into(), name: s["name"].as_str().unwrap_or("Skin").into(), slim: s["model"].as_str() == Some("slim"), builtin: false, remote: false, png });
        }
        out
    }

    fn active_skin_id(&self) -> String {
        let id = self.store["activeSkin"].as_str().unwrap_or("");
        if id.is_empty() { "default".into() } else { id.into() }
    }

    pub fn skins_mount(&mut self, cx: &mut Context<Self>) {
        if self.v.skins.cape_for != self.active_account().filter(|a| !a.2).map(|a| a.0) {
            self.load_capes(cx);
        }
    }

    fn load_capes(&mut self, cx: &mut Context<Self>) {
        let Some((id, name, offline)) = self.active_account() else { return };
        if offline {
            self.v.skins.capes = None;
            self.v.skins.cape_for = None;
            return;
        }
        self.v.skins.cape_for = Some(id.clone());
        self.v.skins.capes = None;
        self.v.skins.cape_err.clear();
        self.fetch_remote_skin(name, cx);
        let creds = self.online_token();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let result = match creds {
                Ok((api, token, _)) => executor.spawn(async move { account_api::capes(&api, &token) }).await,
                Err(e) => Err(e),
            };
            let _ = view.update(cx, |this, cx| {
                if this.active_account().map(|a| a.0) != Some(id) {
                    return;
                }
                match result {
                    Ok(v) => {
                        let capes = v["capes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| Some(Cape { id: c["id"].as_str()?.into(), name: c["name"].as_str().unwrap_or("").into(), png: c["texture"].as_str().and_then(data_png).unwrap_or_default() }))
                            .collect();
                        let choice = v["choice"].as_str().map(String::from);
                        this.v.skins.capes = Some((capes, choice));
                    }
                    Err(e) => {
                        this.v.skins.capes = Some((Vec::new(), None));
                        this.v.skins.cape_err = e;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The cape the account wears (None choice = the default "common" cape, "" = none).
    fn worn_cape(&self) -> Option<Cape> {
        let (capes, choice) = self.v.skins.capes.as_ref()?;
        match choice.as_deref() {
            Some("") => None,
            Some(id) => capes.iter().find(|c| c.id == id).cloned(),
            None => capes.iter().find(|c| c.id == "common").or(capes.first()).cloned(),
        }
    }

    /// Feed the Home viewer the account's skin (home.js `homeSkin`).
    pub fn home_skin(&mut self, cx: &mut Context<Self>) -> Entity<SkinViewer> {
        let acct = self.active_account();
        let (key, png, slim) = match &acct {
            Some((_, name, false)) => match self.remote_skins.get(name) {
                Some(Some(b)) => (format!("remote:{name}"), b.clone(), false),
                None => {
                    self.fetch_remote_skin(name.clone(), cx);
                    let (k, b, s) = self.active_skin_bytes();
                    (k, b, s)
                }
                _ => self.active_skin_bytes(),
            },
            _ => self.active_skin_bytes(),
        };
        let reduce = self.store.pointer("/settings/reduceMotion").and_then(Value::as_bool).unwrap_or(false);
        let v = self.v.skins.home_viewer.clone();
        v.update(cx, |viewer, _| {
            viewer.reduce_motion = reduce;
            viewer.set(&format!("{key}:{:x}", Sha256::digest(&png)), &png, slim, "", None);
        });
        v
    }

    fn cape_card(&self, id: String, name: String, png: Option<Vec<u8>>, on: bool, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.v.skins.cape_busy.is_some();
        let hover_key: SharedString = format!("cape-hover-{id}").into();
        let sway = if busy { 1. } else { crate::ui::motion::cape_sway(&hover_key) };
        let lift = if busy { 0. } else { crate::ui::motion::lift(&hover_key, 3.) };
        let saving = self.v.skins.cape_busy.as_deref() == Some(id.as_str());
        let thumb: AnyElement = match png.as_ref().and_then(|p| {
            let img = image::load_from_memory(p).ok()?.to_rgba8();
            if img.width() == 0 || img.height() == 0 { return None; }
            let legacy = (img.width() as f32 / img.height() as f32 - 22. / 17.).abs() < 0.08;
            let s = (img.width() / if legacy { 22 } else { 64 }).max(1);
            if img.width() < 11 * s || img.height() < 17 * s { return None; }
            let front = image::imageops::crop_imm(&img, s, s, 10 * s, 16 * s).to_image();
            let mut out = Vec::new();
            front.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
            assets::pixel_bytes(&format!("cape:{id}:{:x}", Sha256::digest(p)), &out, 8 / s.min(8))
        }) {
            Some(i) => img(i).w(px(50.)).h(px(80. * sway)).object_fit(gpui::ObjectFit::Fill).into_any_element(),
            None => icon("close", 16., c(DIM2)).into_any_element(),
        };
        let pick = id.clone();
        crate::ui::motion::hoverable(div()
            .id(SharedString::from(format!("cape-{id}"))), hover_key)
            .relative().top(px(lift))
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .pt(px(12.))
            .px(px(8.))
            .pb(px(10.))
            .border_1()
            .border_color(c(if on { 0xe5e5e5 } else { LINE2 }))
            .rounded(px(12.))
            .bg(c(if on { 0x0c0c0c } else { CARD }))
            .when(!busy, |d| d.cursor_pointer().hover(|s| s.border_color(c(0x3a3a3a))).on_click(cx.listener(move |this, _, _, cx| this.choose_cape(pick.clone(), cx))))
            .when(busy && !saving, |d| d.opacity(0.45))
            .child(div().w(px(50.)).h(px(80.)).flex().items_start().justify_center().rounded(px(4.)).bg(c(0x0d0d0d)).overflow_hidden().child(thumb))
            .child(div().text_size(px(13.)).max_w_full().truncate().child(if saving { "Saving…".to_string() } else { name }))
    }

    fn capes_section(&mut self, viewport_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let width = grid_card_width(library_width(viewport_width), 104., 10.);
        let acct = self.active_account();
        let head = |extra: Option<AnyElement>| div().flex().items_center().gap(px(10.)).child(icon("cape", 16., c(FG))).child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Capes")).children(extra);
        let Some((_, name, false)) = acct else {
            return div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(head(None))
                .child(
                    div()
                        .p(px(24.))
                        .border_1()
                        .border_dashed()
                        .border_color(c(LINE2))
                        .rounded(px(14.))
                        .flex()
                        .justify_center()
                        .gap(px(4.))
                        .text_size(px(13.))
                        .text_color(c(DIM))
                        .child("Capes belong to breakmine.com accounts.")
                        .child(link("cape-signin", "Sign in").on_click(cx.listener(|this, _, _, cx| this.go(Route::Signin, cx)))),
                )
                .into_any_element();
        };
        let redeem = div().ml_auto().child(Btn::new("redeem", "Redeem code").sm().icon("gift").icon_size(12.).on_click(cx.listener(|this, _, window, cx| this.redeem_cape(window, cx)))).into_any_element();
        let mut col = div().flex().flex_col().gap(px(12.)).child(head(Some(redeem)));
        match self.v.skins.capes.clone() {
            None => {
                let mut row = div().flex().gap(px(10.));
                for i in 0..3 {
                    let skeleton = div().w(px(width)).h(px(124.)).rounded(px(12.)).border_1().border_color(c(0x111111)).bg(c(0x0a0a0a));
                    let skeleton = if crate::ui::motion::reduced() { skeleton.into_any_element() } else {
                        skeleton.with_animation(SharedString::from(format!("cape-sk-{i}")), gpui::Animation::new(std::time::Duration::from_millis(1400)).repeat(), |el, t| el.opacity(0.6 + 0.4 * (t * std::f32::consts::TAU).sin().abs())).into_any_element()
                    };
                    row = row.child(skeleton);
                }
                col = col.child(row);
            }
            Some((capes, choice)) => {
                if !self.v.skins.cape_err.is_empty() {
                    col = col.child(div().flex().items_center().gap(px(10.)).px(px(12.)).py(px(10.)).border_1().border_color(c(LINE2)).rounded(px(12.)).bg(c(CARD)).text_size(px(13.)).text_color(c(0xd4d4d4)).child(icon("info", 16., c(0xd4d4d4))).child(self.v.skins.cape_err.clone()));
                }
                let is_on = |id: &str| if id.is_empty() { choice.as_deref() == Some("") } else { choice.as_deref() == Some(id) || (choice.is_none() && id == "common") };
                let mut grid = div().flex().flex_wrap().gap(px(10.));
                grid = grid.child(self.cape_card(String::new(), "No cape".into(), None, is_on(""), width, cx));
                for cape in &capes {
                    grid = grid.child(self.cape_card(cape.id.clone(), cape.name.clone(), Some(cape.png.clone()), is_on(&cape.id), width, cx));
                }
                col = col.child(grid);
                if capes.is_empty() {
                    col = col.child(div().text_size(px(12.)).text_color(c(DIM2)).child(format!("{name} has no capes yet. Redeem a code to unlock one.")));
                }
            }
        }
        col.into_any_element()
    }

    pub fn skins_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let acct = self.active_account();
        let online = acct.as_ref().is_some_and(|a| !a.2);
        if online && self.v.skins.cape_for != acct.as_ref().map(|a| a.0.clone()) {
            self.v.skins.sel = None;
            self.load_capes(cx);
        }
        let skins = self.skin_entries();
        let active = self.active_skin_id();
        let sel = self
            .v
            .skins
            .sel
            .as_ref()
            .and_then(|id| skins.iter().find(|s| &s.id == id))
            .or_else(|| if online { skins.iter().find(|s| s.remote) } else { None })
            .or_else(|| skins.iter().find(|s| s.id == active))
            .or_else(|| skins.iter().find(|s| s.builtin))
            .cloned()
            .unwrap();
        let in_use = sel.remote || (!online && sel.id == active);
        let cape = if online { self.worn_cape() } else { None };
        let reduce = self.store.pointer("/settings/reduceMotion").and_then(Value::as_bool).unwrap_or(false);
        let viewer = self.v.skins.viewer.clone();
        viewer.update(cx, |v, _| {
            v.reduce_motion = reduce;
            let cape_key = cape.as_ref().map(|c| format!("{}:{:x}", c.id, Sha256::digest(&c.png))).unwrap_or_default();
            v.set(&format!("{}:{:x}", sel.id, Sha256::digest(&sel.png)), &sel.png, sel.slim, &cape_key, cape.as_ref().map(|c| c.png.as_slice()));
        });

        // Preview column.
        let stage = div()
            .relative()
            .h(px(470.))
            .rounded(px(20.))
            .overflow_hidden()
            .border_1()
            .border_color(c(0x1a1a1a))
            .bg(c(0x04060a))
            .child(div().absolute().inset_0().opacity(0.5).child(assets::rounded_cover(assets::path("assets/game/pano/day_1.webp"), 19., 0.3, 1.0)))
            .child(div().absolute().inset_0().child(viewer))
            .child(div().absolute().top(px(14.)).left(px(14.)).child(sq_btn("rotate", "rotate", SqKind::Dark).on_click(cx.listener(|this, _, _, cx| {
                this.v.skins.viewer.update(cx, |v, cx| { v.turn(); cx.notify(); });
            }))));
        let model_seg = if sel.builtin || sel.remote {
            None
        } else {
            let mut s = seg();
            for (id, label) in [("classic", "Classic"), ("slim", "Slim")] {
                let on = (id == "slim") == sel.slim;
                let sid = sel.id.clone();
                s = s.child(pill(SharedString::from(format!("model-{id}")), label, on, false).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(list) = this.store["skins"].as_array_mut() {
                        if let Some(skin) = list.iter_mut().find(|x| x["id"].as_str() == Some(&sid)) {
                            skin["model"] = json!(id);
                        }
                    }
                    this.save_store();
                    cx.notify();
                })));
            }
            Some(s)
        };
        let busy = self.v.skins.busy;
        let apply_label = if busy { "Uploading…".to_string() } else if sel.remote { "On your account".into() } else if in_use { "In use".into() } else if online { "Upload & use".into() } else { "Use this skin".into() };
        let sel_id = sel.id.clone();
        let mut apply = Btn::new("apply", apply_label).primary().big().disabled(in_use || busy).on_click(cx.listener(move |this, _, _, cx| this.apply_skin(sel_id.clone(), cx)));
        apply = if busy { apply.leading(spinner("apply-spin", c(INK))) } else { apply.icon("check") };
        let preview = div()
            .flex_grow()
            .flex_basis(px(340.))
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(stage)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex().flex_col().gap(px(3.)).mr_auto().child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(sel.name.clone())).child(mono(if sel.remote { "What other players see right now".to_string() } else { format!("{} arms{}", if sel.slim { "Slim" } else { "Classic" }, if sel.builtin { " · comes with the game" } else { "" }) }).text_color(c(DIM))))
                    .children(model_seg)
                    .child(apply),
            );

        // Library.
        let card_width = skin_card_width(f32::from(window.viewport_size().width));
        let mut grid = div().flex().flex_wrap().gap(px(12.));
        let count = skins.len();
        for s in &skins {
            let on = s.id == sel.id;
            let group: SharedString = format!("skin-card-{}", s.id).into();
            let lift_key: SharedString = format!("skin-hover-{}", s.id).into();
            let lift = crate::ui::motion::lift(&lift_key, 3.);
            let thumb = assets::skin_front(&format!("{}:{:x}", s.id, Sha256::digest(&s.png)), &s.png, s.slim);
            let pick = s.id.clone();
            let mut card = crate::ui::motion::hoverable(div()
                .id(SharedString::from(format!("skin-{}", s.id))), lift_key)
                .group(group.clone())
                .relative().top(px(lift))
                .hover(|s| s.shadow(vec![gpui::BoxShadow { color: ca(0x00000080), offset: gpui::point(px(0.), px(8.)), blur_radius: px(20.), spread_radius: px(0.) }]))
                .w(px(card_width))
                .rounded(px(14.))
                .bg(c(CARD2))
                .border_1()
                .border_color(c(if on { 0xe5e5e5 } else { 0x171717 }))
                .child(
                    div()
                        .id(SharedString::from(format!("skin-pick-{}", s.id)))
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .pt(px(8.))
                        .px(px(8.))
                        .pb(px(12.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.v.skins.sel = Some(pick.clone());
                            cx.notify();
                        }))
                        .child(div().h(px(150.)).flex().items_end().justify_center().pb(px(10.)).bg(c(0x0b0b0b)).rounded(px(10.)).children(thumb.map(|t| img(t).w(px(64.)).h(px(128.)))))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(div().truncate().child(s.name.clone()))
                                .when(!online && !s.remote && s.id == active, |d| d.child(div().ml_auto().flex_none().child(icon("check", 16., c(ACCENT))))),
                        ),
                );
            if !s.builtin && !s.remote {
                let (rid, did) = (s.id.clone(), s.id.clone());
                card = card.child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .right(px(12.))
                        .flex()
                        .gap(px(2.))
                        .invisible()
                        .group_hover(group, |st| st.visible())
                        .child(icon_btn(SharedString::from(format!("rn-{}", s.id)), "edit", true, false).bg(c(0x000000)).on_click(cx.listener(move |this, _, window, cx| this.rename_skin(rid.clone(), window, cx))))
                        .child(icon_btn(SharedString::from(format!("rm-{}", s.id)), "trash", true, false).bg(c(0x000000)).on_click(cx.listener(move |this, _, window, cx| this.remove_skin(did.clone(), window, cx)))),
                );
            }
            grid = grid.child(card);
        }
        grid = grid.child(
            div()
                .id("add-skin")
                .w(px(card_width))
                .min_h(px(196.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .border_1()
                .border_dashed()
                .border_color(c(LINE2))
                .rounded(px(14.))
                .text_size(px(13.))
                .text_color(c(DIM))
                .cursor_pointer()
                .hover(|s| s.text_color(c(0xfafafa)).border_color(c(0x3a3a3a)))
                .on_click(cx.listener(|this, _, _, cx| this.import_skin(cx)))
                .child(icon("plus", 16., c(DIM)))
                .child("Add skin")
                .child(div().text_size(px(12.)).text_color(c(DIM2)).mt(px(-4.)).child("64×64 PNG")),
        );
        let note = match &acct {
            Some((_, name, false)) => format!("Skins and capes are saved to {name} on breakmine.com, so everyone sees them in game."),
            _ => "Sign in with a breakmine.com account to upload skins and wear capes. Offline skin selection is saved in the launcher but is not applied in game.".into(),
        };
        let mut lib = div()
            .flex_grow()
            .flex_basis(px(460.))
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(div().flex().items_end().gap(px(10.)).child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Your skins")).child(mono(format!("({count}) · Recently used first")).text_color(c(DIM))))
                    .child(grid),
            )
            .child(self.capes_section(f32::from(window.viewport_size().width), cx))
            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child(note));
        // CSS flex: 999 1 460px: nearly all extra width belongs to the library.
        lib.style().flex_grow = Some(999.);

        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(18.))
            .pt(px(22.))
            .px(px(28.))
            .pb(px(28.))
            .child(div().flex().flex_wrap().items_center().gap(px(10.)).child(pixel_text("Skins", 32.).mr_auto()).child(Btn::new("skin-upload", "Add skin").icon("upload").on_click(cx.listener(|this, _, _, cx| this.import_skin(cx)))))
            .child(div().flex().flex_wrap().items_start().gap(px(24.)).child(preview).child(lib))
    }

    // ── actions ──────────────────────────────────────────────────────────

    fn import_skin(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Add skin".into()) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(src) = paths.into_iter().next() else { return };
            let _ = view.update(cx, |this, cx| {
                let result = (|| {
                    let bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
                    match util::png_size(&bytes) {
                        Some((64, 64)) | Some((64, 32)) => {}
                        Some((w, h)) => return Err(format!("Skins must be 64×64 or 64×32 pixels. This one is {w}×{h}.")),
                        None => return Err("That file isn't a PNG image.".to_string()),
                    }
                    if bytes.len() > 256 * 1024 {
                        return Err("Skins must be under 256 KB.".into());
                    }
                    let id = util::new_id("skin");
                    let file = format!("{id}.png");
                    let dir = crate::launcher_data().join("skins");
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    std::fs::write(dir.join(&file), bytes).map_err(|e| e.to_string())?;
                    let name = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Skin".into());
                    if !this.store["skins"].is_array() {
                        this.store["skins"] = json!([]);
                    }
                    this.store["skins"].as_array_mut().unwrap().push(json!({ "id": id, "name": name, "file": file, "model": "classic", "lastUsed": util::now() }));
                    this.save_store();
                    Ok(id)
                })();
                match result {
                    Ok(id) => {
                        this.v.skins.sel = Some(id);
                        this.info("Skin added", cx);
                    }
                    Err(e) => this.error(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_skin(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(entry) = self.skin_entries().into_iter().find(|s| s.id == id) else { return };
        let online = self.active_account().is_some_and(|a| !a.2);
        let creds = if online { Some(self.online_token()) } else { None };
        if let Some(Err(e)) = &creds {
            return self.error(e.clone(), cx);
        }
        self.v.skins.busy = true;
        cx.notify();
        let png = entry.png.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let result = match creds {
                Some(Ok((api, token, name))) => executor.spawn(async move { account_api::upload_skin(&api, &token, &png).map(|_| Some(name)) }).await,
                _ => Ok(None),
            };
            let _ = view.update(cx, |this, cx| {
                this.v.skins.busy = false;
                match result {
                    Ok(uploaded) => {
                        if let Some(list) = this.store["skins"].as_array_mut() {
                            if let Some(s) = list.iter_mut().find(|s| s["id"].as_str() == Some(&id)) {
                                s["lastUsed"] = json!(util::now());
                            }
                        }
                        this.store["activeSkin"] = json!(if id == "default" { "" } else { &id });
                        this.save_store();
                        match uploaded {
                            Some(name) => {
                                this.remote_skins.insert(name, Some(entry.png.clone()));
                                this.info(format!("{} uploaded to breakmine.com", entry.name), cx);
                            }
                            None => this.info(format!("Now wearing {}", entry.name), cx),
                        }
                    }
                    Err(e) => this.error(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_cape(&mut self, id: String, cx: &mut Context<Self>) {
        let creds = match self.online_token() {
            Ok(c) => c,
            Err(e) => return self.error(e, cx),
        };
        let prev = self.v.skins.capes.as_ref().and_then(|c| c.1.clone());
        if let Some(c) = &mut self.v.skins.capes {
            c.1 = Some(id.clone());
        }
        self.v.skins.cape_busy = Some(id.clone());
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let choice = id.clone();
            let result = executor.spawn(async move { account_api::choose_cape(&creds.0, &creds.1, Some(&choice)) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.skins.cape_busy = None;
                match result {
                    Ok(_) => this.info(if id.is_empty() { "Cape removed" } else { "Cape saved to your account" }, cx),
                    Err(e) => {
                        if let Some(c) = &mut this.v.skins.capes {
                            c.1 = prev;
                        }
                        this.error(e, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn redeem_cape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_modal(
            "Redeem a cape code",
            None,
            vec![FieldSpec::text("code", "Code", "", "ABC-234")],
            "Redeem",
            false,
            None,
            Rc::new(|this, v, _, cx| {
                let code = v[0].1.trim().to_string();
                if code.is_empty() || code.len() > 64 {
                    return this.error("Enter a cape code.", cx);
                }
                let creds = match this.online_token() {
                    Ok(c) => c,
                    Err(e) => return this.error(e, cx),
                };
                let executor = cx.background_executor().clone();
                cx.spawn(async move |view, cx| {
                    let result = executor.spawn(async move { account_api::redeem_cape(&creds.0, &creds.1, &code) }).await;
                    let _ = view.update(cx, |this, cx| {
                        match result {
                            Ok(r) => {
                                let name = r["cape"]["name"].as_str().map(|n| format!("Unlocked {n}!")).unwrap_or_else(|| "Code redeemed!".into());
                                this.info(name, cx);
                                this.v.skins.cape_for = None;
                                this.load_capes(cx);
                            }
                            Err(e) => this.error(e, cx),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }),
            window,
            cx,
        );
    }

    fn rename_skin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.store["skins"].as_array().into_iter().flatten().find(|s| s["id"].as_str() == Some(&id)).and_then(|s| s["name"].as_str()).unwrap_or("").to_string();
        self.open_modal(
            "Rename skin",
            None,
            vec![FieldSpec::text("name", "Name", name, "")],
            "Save",
            false,
            None,
            Rc::new(move |this, v, _, _| {
                let name = v[0].1.trim().to_string();
                if name.is_empty() {
                    return;
                }
                if let Some(list) = this.store["skins"].as_array_mut() {
                    if let Some(s) = list.iter_mut().find(|s| s["id"].as_str() == Some(&id)) {
                        s["name"] = json!(name);
                    }
                }
                this.save_store();
            }),
            window,
            cx,
        );
    }

    fn remove_skin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.store["skins"].as_array().into_iter().flatten().find(|s| s["id"].as_str() == Some(&id)).and_then(|s| s["name"].as_str()).unwrap_or("").to_string();
        self.confirm(
            format!("Delete {name}?"),
            "The skin file is removed from the launcher. This can’t be undone.",
            "Delete",
            Rc::new(move |this, _, _| {
                if let Some(list) = this.store["skins"].as_array_mut() {
                    if let Some(s) = list.iter().find(|s| s["id"].as_str() == Some(&id)) {
                        if let Some(f) = s["file"].as_str().and_then(|f| util::safe_name(f).ok()) {
                            let _ = std::fs::remove_file(crate::launcher_data().join("skins").join(f));
                        }
                    }
                    list.retain(|s| s["id"].as_str() != Some(&id));
                }
                if this.store["activeSkin"].as_str() == Some(&id) {
                    this.store["activeSkin"] = json!("");
                }
                if this.v.skins.sel.as_deref() == Some(&id) {
                    this.v.skins.sel = None;
                }
                this.save_store();
            }),
            window,
            cx,
        );
    }
}


#[cfg(test)]
mod layout_tests {
    use super::{skin_card_width, grid_card_width};

    #[test]
    fn cards_fill_the_same_columns_as_the_web_grid() {
        assert_eq!(skin_card_width(1280.), 133.);
        assert_eq!(skin_card_width(880.), 145.);
        assert_eq!(skin_card_width(700.), 152.);
        assert_eq!(grid_card_width(460., 104., 10.), 107.);
    }
}
