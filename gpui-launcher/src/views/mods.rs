//! Mods (src/js/views/mods.js): browse mods.breakmine.com or manage what's installed in an instance.
use super::icon_image;
use crate::app::{plural, FieldSpec};
use crate::instances::{self, ModInfo};
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, catalog, Launcher};
use gpui::{div, img, prelude::*, px, AnimationExt, AnyElement, ClipboardItem, Context, Entity, FontWeight, SharedString, Subscription, Window};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

pub struct State {
    source: &'static str,
    sort: SharedString,
    query: Entity<TextInput>,
    catalog: Option<Vec<Value>>,
    catalog_error: String,
    loading_catalog: bool,
    mods: Option<Vec<ModInfo>>,
    mods_for: String,
    busy: HashSet<i64>,
    sharing: bool,
    icons: HashMap<String, Option<Arc<gpui::Image>>>,
    _subs: Vec<Subscription>,
}

impl State {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        let query = cx.new(|cx| TextInput::with_placeholder(cx, "Search Breakmine Mods"));
        let subs = vec![cx.subscribe(&query, |_, _, _: &TextEvent, cx| cx.notify())];
        Self { source: "browse", sort: "popular".into(), query, catalog: None, catalog_error: String::new(), loading_catalog: false, mods: None, mods_for: String::new(), busy: HashSet::new(), sharing: false, icons: HashMap::new(), _subs: subs }
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn deprecated(m: &Value) -> bool {
    m["name"].as_str().unwrap_or("").to_lowercase().contains("deprecated")
}

fn mod_tile(size: f32, icon: Option<AnyElement>, pw_size: f32) -> gpui::Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .bg(c(0x111111))
        .overflow_hidden()
        .child(icon.unwrap_or_else(|| patchwork_tint(pw_size, 0x5dd2ea)))
}

impl Launcher {
    fn mods_instance(&self) -> Option<crate::instances::InstanceView> {
        self.current_instance().cloned()
    }

    fn load_mods(&mut self) {
        let Some(c) = self.mods_instance() else { return };
        self.v.mods.mods_for = c.inst.id.clone();
        self.v.mods.mods = Some(instances::list_mods(std::path::Path::new(&c.path), &c.inst));
    }

    fn refresh_after_change(&mut self) {
        self.load_mods();
        self.refresh();
    }

    pub fn mods_mount(&mut self, cx: &mut Context<Self>) {
        self.refresh_instances_in_background(cx);
        self.load_mods();
        if self.v.mods.catalog.is_none() {
            self.load_catalog(cx);
        }
    }

    fn load_catalog(&mut self, cx: &mut Context<Self>) {
        if self.v.mods.loading_catalog {
            return;
        }
        self.v.mods.loading_catalog = true;
        self.v.mods.catalog_error.clear();
        let url = self.store.pointer("/settings/catalogUrl").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("https://mods.breakmine.com/api/mods").to_string();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let result = executor.spawn(async move { catalog::fetch(&url) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.mods.loading_catalog = false;
                match result {
                    Ok(list) => {
                        this.v.mods.catalog = Some(list);
                        this.load_catalog_icons(cx);
                    }
                    Err(e) => {
                        this.v.mods.catalog = Some(Vec::new());
                        this.v.mods.catalog_error = e;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Up to three icon downloads at a time (mods.js `loadCatalogIcons`).
    fn load_catalog_icons(&mut self, cx: &mut Context<Self>) {
        let urls: Vec<String> = self.v.mods.catalog.iter().flatten().filter_map(|m| m["download_url"].as_str().map(String::from)).filter(|u| !self.v.mods.icons.contains_key(u)).collect();
        for u in &urls {
            self.v.mods.icons.insert(u.clone(), None);
        }
        let cache = crate::launcher_data().join("mod-icon-cache");
        let queue = Arc::new(std::sync::Mutex::new(urls));
        for _ in 0..3 {
            let (queue, cache) = (queue.clone(), cache.clone());
            let executor = cx.background_executor().clone();
            cx.spawn(async move |view, cx| loop {
                let next = queue.lock().ok().and_then(|mut q| q.pop());
                let Some(url) = next else { break };
                let (u, c) = (url.clone(), cache.clone());
                let bytes = executor.spawn(async move { catalog::icon(&c, &u).ok().flatten() }).await;
                let _ = view.update(cx, |this, cx| {
                    if let Some(b) = bytes {
                        this.v.mods.icons.insert(url.clone(), assets::pixel_bytes(&format!("modicon:{url}"), &b, 4));
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn installed_for(&self, entry: &Value) -> Option<&ModInfo> {
        let id = entry["id"].as_i64()?;
        self.v.mods.mods.as_ref()?.iter().find(|m| m.catalog_id == Some(id))
    }

    fn mod_icon_el(m: &ModInfo) -> Option<AnyElement> {
        if m.icon_path.is_empty() {
            return None;
        }
        let path = std::path::PathBuf::from(&m.icon_path);
        Some(match assets::pixel_file(&path, 4) {
            Some(i) => img(i).size_full().rounded(px(10.)).object_fit(gpui::ObjectFit::Cover).into_any_element(),
            None => img(path).size_full().rounded(px(10.)).object_fit(gpui::ObjectFit::Cover).into_any_element(),
        })
    }

    fn mod_action(&self, m: &Value, cx: &mut Context<Self>) -> AnyElement {
        let id = m["id"].as_i64().unwrap_or(-1);
        let version = match &m["version"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        if self.v.mods.busy.contains(&id) {
            return Btn::new(SharedString::from(format!("busy-{id}")), "Installing…").disabled(true).leading(spinner(SharedString::from(format!("ms-{id}")), c(FG))).into_any_element();
        }
        let install = |label: String, icon_name: &'static str| {
            let entry = m.clone();
            Btn::new(SharedString::from(format!("inst-{id}")), label).primary().icon(icon_name).on_click(cx.listener(move |this, _, _, cx| this.install_catalog(entry.clone(), cx))).into_any_element()
        };
        match self.installed_for(m) {
            None => install("Install".into(), "down"),
            Some(have) if !have.catalog_version.is_empty() && have.catalog_version != version => install(format!("Update to {version}"), "rotate"),
            Some(have) => {
                let mid = have.id.clone();
                Btn::new(SharedString::from(format!("rm-{id}")), "Installed").leading(icon("check", 16., c(ACCENT))).on_click(cx.listener(move |this, _, window, cx| this.uninstall_mod(mid.clone(), window, cx))).into_any_element()
            }
        }
    }

    fn browse_view(&mut self, inst: &crate::instances::InstanceView, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sort_id: SharedString = "mods-sort".into();
        let open = self.dd_open.as_ref() == Some(&sort_id);
        let tid = sort_id.clone();
        let sort = Dropdown::new(sort_id, vec![DdOpt::new("popular", "Most downloaded"), DdOpt::new("new", "Newest"), DdOpt::new("name", "Name (A–Z)")], self.v.mods.sort.clone(), open)
            .width(170.)
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
            .on_pick(cx.listener(|this, v: &SharedString, _, cx| {
                this.v.mods.sort = v.clone();
                this.dd_open = None;
                cx.notify();
            }));
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(div().flex_1().min_w(px(260.)).child(search_box(&self.v.mods.query, window, cx, true)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(30.))
                    .px(px(12.))
                    .border_1()
                    .border_color(c(LINE2))
                    .rounded(px(8.))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(c(0xd4d4d4))
                    .child(div().size(px(6.)).bg(c(ACCENT)))
                    .child(format!("{} · {}", inst.inst.name, self.build_label(&inst.inst))),
            )
            .child(div().text_size(px(13.)).text_color(c(DIM)).child("Sort"))
            .child(sort);

        let Some(catalog) = self.v.mods.catalog.clone() else {
            let mut sk = div().flex().flex_col().gap(px(8.));
            for i in 0..4 {
                let skeleton = div().h(px(92.)).border_1().border_color(c(0x111111)).rounded(px(14.)).bg(c(0x0a0a0a));
                let skeleton = if crate::ui::motion::reduced() { skeleton.into_any_element() } else {
                    skeleton.with_animation(SharedString::from(format!("sk-{i}")), gpui::Animation::new(std::time::Duration::from_millis(1400)).repeat(), |el, t| el.opacity(0.6 + 0.4 * (t * std::f32::consts::TAU).sin().abs())).into_any_element()
                };
                sk = sk.child(skeleton);
            }
            return div().flex().flex_col().gap(px(18.)).child(head).child(sk).into_any_element();
        };
        let q = self.v.mods.query.read(cx).value().trim().to_lowercase();
        let mut list: Vec<Value> = catalog
            .into_iter()
            .filter(|m| q.is_empty() || format!("{} {} {}", m["name"].as_str().unwrap_or(""), m["author"].as_str().unwrap_or(""), m["description"].as_str().unwrap_or("")).to_lowercase().contains(&q))
            .collect();
        match self.v.mods.sort.as_ref() {
            "popular" => list.sort_by(|a, b| b["downloads"].as_u64().unwrap_or(0).cmp(&a["downloads"].as_u64().unwrap_or(0))),
            "new" => list.sort_by(|a, b| b["created_at"].to_string().cmp(&a["created_at"].to_string())),
            _ => list.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))),
        }
        list.sort_by_key(deprecated);

        let mut results = div().flex().flex_col().gap(px(8.));
        if list.is_empty() && self.v.mods.catalog_error.is_empty() {
            results = results.child(empty_box(format!("No mods match “{q}”.")));
        }
        for m in &list {
            let installed = self.installed_for(m).cloned();
            let icon_el = installed
                .as_ref()
                .and_then(Self::mod_icon_el)
                .or_else(|| m["download_url"].as_str().and_then(|u| self.v.mods.icons.get(u).cloned().flatten()).map(|i| img(i).size_full().rounded(px(10.)).object_fit(gpui::ObjectFit::Cover).into_any_element()));
            let version = match &m["version"] {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            let mut tags = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(tag(format!("v{version}")))
                .child(tag(m["file_size_formatted"].as_str().unwrap_or("").to_string()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .bg(c(0x111111))
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(c(MUTED))
                        .child(icon("down", 10., c(MUTED)))
                        .child(thousands(m["downloads"].as_u64().unwrap_or(0))),
                );
            if let Some(pw) = m["min_patchwork"].as_str().filter(|s| !s.is_empty()) {
                tags = tags.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .bg(c(0x111111))
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(c(0x5dd2ea))
                        .child(patchwork_tint(10., 0x5dd2ea))
                        .child(format!("Patchwork {pw}+")),
                );
            }
            if let Some(url) = m["url"].as_str().filter(|s| !s.is_empty()) {
                let url = url.replacen("http:", "https:", 1);
                tags = tags.child(link(SharedString::from(format!("page-{}", m["id"])), "Mod page").text_size(px(12.)).child(icon("external", 10., c(0xd4d4d4))).on_click(move |_, _, cx| cx.open_url(&url)));
            }
            let row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(14.))
                .p(px(14.))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(14.))
                .bg(c(CARD2))
                .hover(|s| s.border_color(c(0x262626)))
                .when(deprecated(m), |d| d.opacity(0.55))
                .child(mod_tile(52., icon_el, 26.))
                .child(
                    div()
                        .flex_grow()
                        .flex_shrink()
                        .flex_basis(px(320.))
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_end()
                                .gap(px(8.))
                                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(m["name"].as_str().unwrap_or("").to_string()))
                                .child(div().text_size(px(13.)).text_color(c(DIM)).child(format!("by {}", m["author"].as_str().unwrap_or("")))),
                        )
                        .child(div().text_size(px(13.)).text_color(c(DIM)).line_height(gpui::relative(1.45)).line_clamp(3).child(m["description"].as_str().unwrap_or("").to_string()))
                        .child(tags),
                )
                .child(self.mod_action(m, cx));
            results = results.child(row);
        }
        let mut col = div().flex().flex_col().gap(px(18.)).child(head);
        if !self.v.mods.catalog_error.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(10.))
                    .border_1()
                    .border_color(c(LINE2))
                    .rounded(px(12.))
                    .bg(c(CARD))
                    .text_size(px(13.))
                    .text_color(c(0xd4d4d4))
                    .child(icon("info", 16., c(0xd4d4d4)))
                    .child(self.v.mods.catalog_error.clone())
                    .child(div().ml_auto().child(Btn::new("retry", "Retry").sm().on_click(cx.listener(|this, _, _, cx| {
                        this.v.mods.catalog = None;
                        this.load_catalog(cx);
                    })))),
            );
        }
        col.child(results).into_any_element()
    }

    fn installed_view(&mut self, inst: &crate::instances::InstanceView, cx: &mut Context<Self>) -> AnyElement {
        let Some(mods) = self.v.mods.mods.clone() else {
            return div().text_size(px(13.)).text_color(c(DIM)).child("Loading…").into_any_element();
        };
        let catalog = self.v.mods.catalog.clone().unwrap_or_default();
        let mut list = div().flex().flex_col().border_1().border_color(c(LINE)).rounded(px(14.)).bg(c(CARD)).overflow_hidden();
        let count = mods.len();
        for (i, m) in mods.iter().enumerate() {
            let entry = m.catalog_id.and_then(|id| catalog.iter().find(|x| x["id"].as_i64() == Some(id))).cloned();
            let entry_version = entry.as_ref().map(|e| match &e["version"] {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            });
            let upd = entry_version.as_ref().is_some_and(|v| !m.catalog_version.is_empty() && *v != m.catalog_version);
            let sub = [m.author.clone(), if m.version.is_empty() { String::new() } else { format!("v{}", m.version) }, if m.catalog_id.is_some() { "mods.breakmine.com".into() } else { "local".into() }]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let (tid, enabled, rid) = (m.id.clone(), m.enabled, m.id.clone());
            let share_id = m.id.clone();
            let share_name = m.name.clone();
            let mut name = div().flex().items_center().gap(px(6.)).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(m.name.clone());
            if upd {
                name = name.child(tag(format!("Update {}", entry_version.clone().unwrap_or_default())).bg(c(ACCENT)).text_color(c(INK)));
            }
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .when(i + 1 < count, |d| d.border_b_1().border_color(c(0x101010)))
                    .child(mod_tile(40., Self::mod_icon_el(m), 18.))
                    .child(div().flex().flex_col().gap(px(2.)).min_w_0().flex_1().child(name).child(mono(sub).text_color(c(DIM)).truncate()))
                    .when(upd, |d| {
                        let e = entry.clone().unwrap();
                        d.child(Btn::new(SharedString::from(format!("upd-{i}")), "Update").primary().sm().on_click(cx.listener(move |this, _, _, cx| this.install_catalog(e.clone(), cx))))
                    })
                    .child(sq_btn(SharedString::from(format!("mshare-{i}")), "copy", SqKind::Default).on_click(cx.listener(move |this, _, window, cx| this.share_mod(share_id.clone(), share_name.clone(), window, cx))))
                    .child(switch(SharedString::from(format!("msw-{i}")), m.enabled).on_click(cx.listener(move |this, _, _, cx| {
                        let Some(id) = this.mods_instance().map(|c| c.inst.id) else { return };
                        if let Err(e) = instances::set_mod_enabled(&this.game_dir, &id, &tid, !enabled) {
                            this.error(e, cx);
                        }
                        this.refresh_after_change();
                        cx.notify();
                    })))
                    .child(sq_btn(SharedString::from(format!("mrm-{i}")), "trash", SqKind::Default).on_click(cx.listener(move |this, _, window, cx| this.uninstall_mod(rid.clone(), window, cx)))),
            );
        }
        if mods.is_empty() {
            list = list.child(div().p(px(28.)).flex().justify_center().text_size(px(13.)).text_color(c(DIM)).child(format!("Nothing installed in {}. Browse Breakmine Mods, or install a .zip, .jph or mod folder.", inst.inst.name)));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(list)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(Btn::new("open-mods", "Open mods/").sm().icon("folder").on_click(cx.listener(|this, _, _, cx| {
                        if let Some(c) = this.mods_instance() {
                            let p = std::path::PathBuf::from(&c.path).join("mods");
                            let _ = std::fs::create_dir_all(&p);
                            cx.open_with_system(&p);
                        }
                    })))
                    .child(div().ml_auto().text_size(px(12.)).text_color(c(DIM2)).child("Changes apply the next time the game starts. Texture packs are installed in-game, under Texture Packs on the title screen.")),
            )
            .into_any_element()
    }

    fn source_tab(&self, id: &'static str, label: &'static str, sub: String, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.v.mods.source == id;
        let badge = div().size(px(36.)).flex_none().flex().items_center().justify_center().rounded(px(8.)).bg(c(0x111111)).child(if id == "browse" {
            img(assets::path("assets/logo.png")).size(px(24.)).into_any_element()
        } else {
            icon("cube", 16., c(if on { ACCENT } else { DIM })).into_any_element()
        });
        div()
            .id(id)
            .flex_1()
            .min_h(px(56.))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(if on { c(0x2a2a2a) } else { ca(0x00000000) })
            .when(on, |d| d.bg(c(0x141414)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.v.mods.source = id;
                cx.notify();
            }))
            .child(badge)
            .child(div().flex().flex_col().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(label)).child(div().text_size(px(12.)).text_color(c(DIM)).child(sub)))
    }

    pub fn mods_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let page = div().flex_1().flex().flex_col().gap(px(18.)).pt(px(22.)).px(px(28.)).pb(px(28.));
        let Some(inst) = self.mods_instance() else {
            return page.child(pixel_text("Mods", 32.)).child(div().p(px(28.)).flex().justify_center().text_size(px(13.)).text_color(c(DIM)).child("Create an instance first.")).into_any_element();
        };
        if self.v.mods.mods_for != inst.inst.id {
            self.load_mods();
        }
        let prof_id: SharedString = "mods-prof".into();
        let open = self.dd_open.as_ref() == Some(&prof_id);
        let options: Vec<DdOpt> = self
            .instances
            .iter()
            .map(|i| {
                let mut o = DdOpt::new(i.inst.id.clone(), i.inst.name.clone()).sub(format!("{}{}", self.build_label(&i.inst), if i.mods_count > 0 { format!(" · {}", plural(i.mods_count, "mod")) } else { String::new() }));
                if let Some(im) = icon_image(&i.inst) {
                    o = o.img(im, true);
                }
                o
            })
            .collect();
        let tid = prof_id.clone();
        let prof = Dropdown::new(prof_id, options, inst.inst.id.clone(), open)
            .width(250.)
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
            .on_pick(cx.listener(|this, v: &SharedString, _, cx| {
                this.selected = v.to_string();
                if let Some(s) = this.store.get_mut("settings").and_then(|s| s.as_object_mut()) {
                    s.insert("selectedInstance".into(), v.to_string().into());
                }
                this.save_store();
                this.v.mods.mods = None;
                this.dd_open = None;
                this.load_mods();
                cx.notify();
            }));
        let n = self.v.mods.mods.as_ref().map(|m| m.len()).unwrap_or(0);
        let body = if self.v.mods.source == "installed" { self.installed_view(&inst, cx) } else { self.browse_view(&inst, window, cx) };
        page.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(div().flex().flex_col().gap(px(6.)).child(pixel_text("Mods", 32.)).child(img(assets::path("assets/game/patchwork_wordmark.png")).h(px(14.)).w(px(100.)).object_fit(gpui::ObjectFit::Contain).opacity(0.85)))
                .child(div().ml(px(8.)).mr_auto().child(prof))
                .child(Btn::new("mod-shared", "Shared mod").on_click(cx.listener(|this, _, window, cx| this.install_shared_dialog(window, cx))))
                .child(Btn::new("mod-folder", "Mod folder").icon("folder").on_click(cx.listener(|this, _, _, cx| this.install_mod_from_disk(true, cx))))
                .child(Btn::new("mod-zip", "Install mod").icon("upload").on_click(cx.listener(|this, _, _, cx| this.install_mod_from_disk(false, cx)))),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .p(px(6.))
                .border_1()
                .border_color(c(0x1a1a1a))
                .rounded(px(14.))
                .bg(c(0x060606))
                .child(self.source_tab("browse", "Breakmine Mods", "mods.breakmine.com".into(), cx))
                .child(self.source_tab("installed", "Installed", if n > 0 { format!("{n} in this instance") } else { "Nothing yet".into() }, cx)),
        )
        .child(body)
        .into_any_element()
    }

    // ── actions ──────────────────────────────────────────────────────────

    fn share_mod(&mut self, id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.v.mods.sharing { return self.info("A mod share is already in progress.", cx); }
        let Some(inst) = self.mods_instance() else { return };
        self.open_modal(format!("Share {name}?"), Some("The mod is uploaded to Breakmine Mods. Anyone with the code can download it for 5 minutes."), vec![], "Share and copy code", false, None,
            Rc::new(move |this, _, _, cx| {
                if this.v.mods.sharing { return; }
                let Ok(safe) = crate::util::safe_name(&id) else { return this.error("Invalid mod ID.", cx) };
                let path = std::path::PathBuf::from(&inst.path).join("mods").join(safe);
                this.v.mods.sharing = true;
                this.info("Uploading mod...", cx);
                let executor = cx.background_executor().clone();
                cx.spawn(async move |view, cx| {
                    let result = executor.spawn(async move { crate::mod_share::upload(&crate::mod_share::package(&path)?) }).await;
                    let _ = view.update(cx, |this, cx| {
                        this.v.mods.sharing = false;
                        match result {
                            Ok(code) => { cx.write_to_clipboard(ClipboardItem::new_string(code)); this.info("Share code copied. It expires in 5 minutes.", cx); }
                            Err(error) => this.error(error, cx),
                        }
                        cx.notify();
                    });
                }).detach();
            }), window, cx);
    }

    fn install_shared_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inst) = self.mods_instance() else { return };
        self.open_modal("Install shared mod", Some("Enter a share code. Codes expire after 5 minutes."), vec![FieldSpec::text("code", "Share code", "", "Share code")], "Install", false, None,
            Rc::new(move |this, values, _, cx| {
                let code = values[0].1.trim().to_string();
                let game_dir = this.game_dir.clone();
                let instance_id = inst.inst.id.clone();
                this.info("Downloading shared mod...", cx);
                let executor = cx.background_executor().clone();
                cx.spawn(async move |view, cx| {
                    let result = executor.spawn(async move {
                        let bytes = crate::mod_share::download(&code)?;
                        instances::install_shared(&game_dir, &instance_id, &bytes)
                    }).await;
                    let _ = view.update(cx, |this, cx| {
                        match result {
                            Ok(id) => { this.info(format!("Installed {id}"), cx); this.v.mods.source = "installed"; }
                            Err(error) => this.error(error, cx),
                        }
                        this.refresh_after_change();
                        cx.notify();
                    });
                }).detach();
            }), window, cx);
    }

    fn install_catalog(&mut self, entry: Value, cx: &mut Context<Self>) {
        let Some(inst) = self.mods_instance() else { return };
        let id = entry["id"].as_i64().unwrap_or(-1);
        let Some(url) = entry["download_url"].as_str().map(String::from) else { return self.error("This mod has no download.", cx) };
        let version = match &entry["version"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        let name = entry["name"].as_str().unwrap_or("Mod").to_string();
        self.v.mods.busy.insert(id);
        cx.notify();
        let mods_dir = std::path::PathBuf::from(&inst.path).join("mods");
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let result = executor.spawn(async move { catalog::install(&mods_dir, &url, id, &version) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.mods.busy.remove(&id);
                match result {
                    Ok(_) => this.info(format!("{name} installed in {}", inst.inst.name), cx),
                    Err(e) => this.error(e, cx),
                }
                this.refresh_after_change();
                cx.notify();
            });
        })
        .detach();
    }

    fn uninstall_mod(&mut self, mod_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inst) = self.mods_instance() else { return };
        let name = self.v.mods.mods.iter().flatten().find(|m| m.id == mod_id).map(|m| m.name.clone()).unwrap_or(mod_id.clone());
        self.confirm(
            format!("Remove {name}?"),
            &format!("The mod is deleted from {}. Data it saved in your worlds stays.", inst.inst.name),
            "Remove",
            Rc::new(move |this, _, cx| {
                if let Err(e) = instances::remove_mod(&this.game_dir, &inst.inst.id, &mod_id) {
                    this.error(e, cx);
                }
                this.refresh_after_change();
            }),
            window,
            cx,
        );
    }

    /// "Install mod" (.zip / .jph) or "Mod folder" (an unpacked mod you're developing).
    fn install_mod_from_disk(&mut self, folder: bool, cx: &mut Context<Self>) {
        let Some(inst) = self.mods_instance() else { return };
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some(if folder { "Choose an unpacked mod folder".into() } else { "Choose a mod (.zip or .jph)".into() }),
        });
        let game_dir = self.game_dir.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let id = inst.inst.id.clone();
            let result = executor
                .spawn(async move { if folder { instances::install_local_folder(&game_dir, &id, &path) } else { instances::install_archive(&game_dir, &id, &path) } })
                .await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(mod_id) => {
                        this.info(format!("Installed {mod_id}"), cx);
                        this.v.mods.source = "installed";
                    }
                    Err(e) => this.error(e, cx),
                }
                this.refresh_after_change();
                cx.notify();
            });
        })
        .detach();
    }
}
