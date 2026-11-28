//! Create / Mod Maker (src/js/views/create.js): projects, blocks, items, recipes, commands, code, patches.
use super::icon_image;
use crate::app::FieldSpec;
use crate::creator::{generate as g, pixel, projects, vanilla::VANILLA};
use crate::skin3d::CubeView;
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, instances, util, Launcher};
use base64::Engine;
use gpui::{
    div, img, prelude::*, px, AnimationExt, AnyElement, Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString, Subscription, Window,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

const SNIPPETS: [(&str, &str); 3] = [
    ("Chat message", "minecraft.addMessageToChat(\"§eHello from my mod!\");"),
    ("Command", "modLoader.registerCommand(\"ping\", \"\", \"Replies with pong\", (mc, args) => {\n    mc.addMessageToChat(\"§7pong\");\n    return true;\n});"),
    ("Every tick", "modLoader.registerRenderHook('onTick', (context) => {\n    // runs every game tick\n});"),
];

/// Text fields bound to the project: (field id, multi-line rows).
const FIELDS: [(&str, usize); 14] = [
    ("name", 0),
    ("modId", 0),
    ("author", 0),
    ("version", 0),
    ("license", 0),
    ("description", 2),
    ("x.name", 0),
    ("x.key", 0),
    ("cmd.name", 0),
    ("cmd.description", 0),
    ("cmd.reply", 4),
    ("code", 16),
    ("patch.file", 0),
    ("patch.source", 18),
];

pub struct State {
    projects: Option<Vec<Value>>,
    cur: Option<String>,
    tab: &'static str,
    sel: HashMap<&'static str, usize>,
    status: &'static str,
    errors: Vec<String>,
    picker: Option<usize>,
    code_open: HashSet<String>,
    inst: Option<String>,
    busy: bool,
    patch: usize,
    save_seq: u64,
    pixel: HashMap<String, pixel::Editor>,
    /// Text inputs by field id; filled from the project when the selection changes.
    inputs: HashMap<&'static str, Entity<TextInput>>,
    query: Entity<TextInput>,
    hex: Entity<TextInput>,
    bound: String,
    /// Top-left of each pixel canvas on screen, recorded while painting (for mouse → cell).
    pixel_origin: Rc<std::cell::RefCell<HashMap<String, gpui::Point<gpui::Pixels>>>>,
    pub cube: Entity<CubeView>,
    pub empty_cube: Entity<CubeView>,
    _subs: Vec<Subscription>,
}

impl State {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        let mut inputs = HashMap::new();
        let mut subs = Vec::new();
        for (key, rows) in FIELDS {
            let input = cx.new(|cx| {
                let mut t = TextInput::with_placeholder(cx, "");
                if rows > 0 {
                    t.set_multiline(rows);
                }
                t
            });
            subs.push(cx.subscribe(&input, move |this: &mut Launcher, input, ev: &TextEvent, cx| {
                if matches!(ev, TextEvent::Changed) {
                    let v = input.read(cx).value().to_string();
                    this.mk_input(key, v, cx);
                }
            }));
            inputs.insert(key, input);
        }
        for (key, ph) in [("description", "What does your mod add?"), ("patch.file", "net/minecraft/client/.../GameSettings.js")] {
            inputs[key].update(cx, |t, _| t.set_placeholder(ph));
        }
        let query = cx.new(|cx| TextInput::with_placeholder(cx, "Search blocks and items"));
        subs.push(cx.subscribe(&query, |_, _, _: &TextEvent, cx| cx.notify()));
        let hex = cx.new(|cx| TextInput::with_placeholder(cx, "#7cc84a"));
        subs.push(cx.subscribe(&hex, |this: &mut Launcher, input, ev: &TextEvent, cx| {
            let v = input.read(cx).value().trim().to_string();
            let ok = v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit());
            if ok || matches!(ev, TextEvent::Submit) {
                if let Some(id) = this.texture_id() {
                    if ok {
                        this.v.create.pixel.entry(id).or_default().color = v.to_lowercase();
                        cx.notify();
                    }
                }
            }
        }));
        let empty_cube = cx.new(|_| CubeView::new(88.));
        let grass = |n: &str| std::fs::read(assets::path(&format!("assets/game/blocks/{n}.png"))).unwrap_or_default();
        let (side, top, bot) = (grass("breakmine_grass_side"), grass("breakmine_grass_top"), grass("breakmine_dirt"));
        // The CSS tints the grass top with background-blend-mode: multiply #7cbd6b.
        let top = tint_png(&top, [0x7c, 0xbd, 0x6b]).unwrap_or(top);
        empty_cube.update(cx, |c, _| c.set("grass", &side, &top, &bot));
        Self {
            projects: None,
            cur: None,
            // Debug aid, like BM_ROUTE: open on a Mod Maker tab.
            tab: match std::env::var("BM_CREATE_TAB").as_deref() {
                Ok("blocks") => "blocks",
                Ok("items") => "items",
                Ok("recipes") => "recipes",
                Ok("code") => "code",
                _ => "details",
            },
            sel: HashMap::new(),
            status: "saved",
            errors: Vec::new(),
            picker: None,
            code_open: HashSet::new(),
            inst: None,
            busy: false,
            patch: 0,
            save_seq: 0,
            pixel: HashMap::new(),
            inputs,
            query,
            hex,
            bound: String::new(),
            pixel_origin: Rc::new(std::cell::RefCell::new(HashMap::new())),
            cube: cx.new(|_| CubeView::new(64.)),
            empty_cube,
            _subs: subs,
        }
    }
}

fn tint_png(png: &[u8], tint: [u8; 3]) -> Option<Vec<u8>> {
    let mut img = image::load_from_memory(png).ok()?.to_rgba8();
    for p in img.pixels_mut() {
        for i in 0..3 {
            p.0[i] = ((p.0[i] as u32 * tint[i] as u32) / 255) as u8;
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn data_bytes(url: &str) -> Option<Vec<u8>> {
    let b64 = url.split_once(',').map(|(_, b)| b)?;
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()
}

/// A texture data URL as a sharp image element, or the empty-texture placeholder.
fn tex(url: &str, size: f32, radius: f32) -> AnyElement {
    match data_bytes(url).and_then(|b| assets::pixel_bytes(&format!("tex:{}", util_hash(url)), &b, 8)) {
        Some(i) => img(i).size(px(size)).rounded(px(radius)).flex_none().into_any_element(),
        None => div().size(px(size)).rounded(px(radius)).bg(c(0x0d0d0d)).flex_none().flex().items_center().justify_center().child(icon("cube", 16., c(DIM2))).into_any_element(),
    }
}

fn util_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn list_key(kind: &str) -> &'static str {
    match kind {
        "block" => "blocks",
        "item" => "items",
        "recipe" => "recipes",
        _ => "commands",
    }
}

impl Launcher {
    fn mk(&self) -> Option<&Value> {
        let id = self.v.create.cur.as_ref()?;
        self.v.create.projects.as_ref()?.iter().find(|p| p["id"].as_str() == Some(id))
    }
    fn mk_mut(&mut self) -> Option<&mut Value> {
        let id = self.v.create.cur.clone()?;
        self.v.create.projects.as_mut()?.iter_mut().find(|p| p["id"].as_str() == Some(&id))
    }
    fn sel_of(&self, kind: &str) -> usize {
        *self.v.create.sel.get(kind).unwrap_or(&0)
    }
    fn tab_kind(&self) -> &'static str {
        match self.v.create.tab {
            "blocks" => "block",
            "items" => "item",
            "recipes" => "recipe",
            _ => "command",
        }
    }
    /// Index of the selected thing of `kind` (clamped), if any.
    fn selected_index(&self, kind: &str) -> Option<usize> {
        let len = self.mk()?[list_key(kind)].as_array()?.len();
        (len > 0).then(|| self.sel_of(kind).min(len - 1))
    }
    fn selected_mut(&mut self, kind: &str) -> Option<&mut Value> {
        let i = self.selected_index(kind)?;
        self.mk_mut()?[list_key(kind)].as_array_mut()?.get_mut(i)
    }
    fn texture_id(&self) -> Option<String> {
        match self.v.create.tab {
            "blocks" => self.selected_index("block").map(|i| format!("block:{i}")),
            "items" => self.selected_index("item").map(|i| format!("item:{i}")),
            _ => None,
        }
    }

    pub fn create_mount(&mut self, cx: &mut Context<Self>) {
        if self.v.create.projects.is_none() {
            let list = projects::list(&crate::launcher_data());
            if self.v.create.cur.is_none() {
                self.v.create.cur = list.first().and_then(|p| p["id"].as_str().map(String::from));
            }
            self.v.create.projects = Some(list);
            self.v.create.bound.clear();
        }
        self.refresh();
        let _ = cx;
    }

    /// Fill the text inputs from the project when the project, tab or selection changed.
    fn bind_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.mk().cloned() else { return };
        let kind = self.tab_kind();
        let key = format!("{}:{}:{}:{}", s(&p["id"]), self.v.create.tab, self.selected_index(kind).unwrap_or(0), self.v.create.patch);
        if key == self.v.create.bound {
            return;
        }
        self.v.create.bound = key;
        let x = self.selected_index(kind).map(|i| p[list_key(kind)][i].clone()).unwrap_or(Value::Null);
        let patch = p["patches"].as_array().and_then(|l| l.get(self.v.create.patch.min(l.len().saturating_sub(1)))).cloned().unwrap_or(Value::Null);
        let values: [(&str, String); 14] = [
            ("name", s(&p["name"])),
            ("modId", s(&p["modId"])),
            ("author", s(&p["author"])),
            ("version", s(&p["version"])),
            ("license", s(&p["license"])),
            ("description", s(&p["description"])),
            ("x.name", s(&x["name"])),
            ("x.key", s(&x["key"])),
            ("cmd.name", s(&x["name"])),
            ("cmd.description", s(&x["description"])),
            ("cmd.reply", s(&x["reply"])),
            ("code", s(&p["code"])),
            ("patch.file", s(&patch["file"])),
            ("patch.source", s(&patch["source"])),
        ];
        for (k, v) in values {
            self.v.create.inputs[k].update(cx, |t, cx| {
                if t.value() != v {
                    t.set_text(v);
                    cx.notify();
                }
            });
        }
    }

    /// create.js `inputs.p / x / patch`: a text field changed.
    fn mk_input(&mut self, key: &'static str, v: String, cx: &mut Context<Self>) {
        let kind = self.tab_kind();
        let patch_i = self.v.create.patch;
        let Some(p) = self.mk_mut() else { return };
        match key {
            "modId" => {
                let val: String = v.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '_' }).collect();
                let old = s(&p["modId"]);
                // Keep block/item ids that still start with the old prefix in step.
                for list in ["blocks", "items"] {
                    for x in p[list].as_array_mut().into_iter().flatten() {
                        let k = s(&x["key"]);
                        if let Some(rest) = k.strip_prefix(&format!("{old}_")) {
                            x["key"] = json!(format!("{val}_{rest}"));
                        }
                    }
                }
                p["modId"] = json!(val.clone());
                if val != v {
                    self.v.create.inputs["modId"].update(cx, |t, cx| {
                        t.set_text(val);
                        cx.notify();
                    });
                }
            }
            "name" | "author" | "version" | "license" | "description" | "code" => p[key] = json!(v),
            "x.name" | "x.key" => {
                let Some(i) = self.selected_index(kind) else { return };
                let p = self.mk_mut().unwrap();
                let lk = list_key(kind);
                let old = s(&p[lk][i]["key"]);
                if key == "x.name" {
                    p[lk][i]["name"] = json!(v);
                    // The id follows the name until it's edited by hand.
                    if (kind == "block" || kind == "item") && !p[lk][i]["keyEdited"].as_bool().unwrap_or(false) {
                        let nk = g::to_key(&v);
                        if !nk.is_empty() {
                            p[lk][i]["key"] = json!(nk.clone());
                            for r in p["recipes"].as_array_mut().into_iter().flatten() {
                                if s(&r["result"]) == old {
                                    r["result"] = json!(nk.clone());
                                }
                            }
                            self.v.create.inputs["x.key"].update(cx, |t, cx| {
                                t.set_text(nk);
                                cx.notify();
                            });
                        }
                    }
                } else {
                    let nk: String = v.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '_' }).collect();
                    p[lk][i]["keyEdited"] = json!(true);
                    p[lk][i]["key"] = json!(nk.clone());
                    for r in p["recipes"].as_array_mut().into_iter().flatten() {
                        if s(&r["result"]) == old {
                            r["result"] = json!(nk.clone());
                        }
                    }
                    if nk != v {
                        self.v.create.inputs["x.key"].update(cx, |t, cx| {
                            t.set_text(nk);
                            cx.notify();
                        });
                    }
                }
            }
            "cmd.name" | "cmd.description" | "cmd.reply" => {
                let Some(x) = self.selected_mut("command") else { return };
                let field = &key[4..];
                let val = if field == "name" { v.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect() } else { v.clone() };
                x[field] = json!(val.clone());
                if field == "name" && val != v {
                    self.v.create.inputs["cmd.name"].update(cx, |t, cx| {
                        t.set_text(val);
                        cx.notify();
                    });
                }
            }
            "patch.file" | "patch.source" => {
                if let Some(x) = p["patches"].as_array_mut().and_then(|l| l.get_mut(patch_i)) {
                    x[&key[6..]] = json!(v);
                }
            }
            _ => {}
        }
        self.queue_save(cx);
        cx.notify();
    }

    // ── saving ───────────────────────────────────────────────────────────

    fn queue_save(&mut self, cx: &mut Context<Self>) {
        self.v.create.status = "dirty";
        self.v.create.save_seq += 1;
        let seq = self.v.create.save_seq;
        let timer = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            timer.timer(Duration::from_millis(700)).await;
            let _ = view.update(cx, |this, cx| {
                if this.v.create.save_seq == seq {
                    this.save_now(cx);
                }
            });
        })
        .detach();
    }

    /// Save project.json and regenerate the mod folder (create.js `saveNow`).
    fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.mk_mut() else { return false };
        p["updated"] = json!(util::now());
        // The mod icon: a custom picture, or the first texture scaled to 64×64.
        let custom = s(&p["customIcon"]);
        let icon = if !custom.is_empty() {
            custom
        } else {
            let first = p["blocks"].as_array().and_then(|l| l.first()).or_else(|| p["items"].as_array().and_then(|l| l.first())).map(|x| s(&x["texture"])).unwrap_or_default();
            if first.is_empty() {
                String::new()
            } else {
                let img = pixel::decode(&first);
                pixel::encode(&image::imageops::resize(&img, 64, 64, image::imageops::FilterType::Nearest))
            }
        };
        p["icon"] = json!(icon);
        let p = p.clone();
        self.v.create.errors = g::validate(&p);
        let result = projects::save(&crate::launcher_data(), &p, g::generate(&p));
        match result {
            Ok(_) => {
                self.v.create.status = "saved";
                cx.notify();
                true
            }
            Err(e) => {
                self.v.create.status = "error";
                self.error(e, cx);
                false
            }
        }
    }

    // ── pieces ───────────────────────────────────────────────────────────

    fn status_badge(&self) -> impl IntoElement {
        let errs = self.mk().map(g::validate).unwrap_or_default();
        let (kind, text) = if !errs.is_empty() {
            ("warn", format!("{} to fix", errs.len()))
        } else {
            match self.v.create.status {
                "saving" => ("", "Saving…".to_string()),
                "dirty" => ("", "Editing…".to_string()),
                "error" => ("warn", "Not saved".to_string()),
                _ => ("ok", "Saved".to_string()),
            }
        };
        let (fg, bg) = match kind {
            "ok" => (0x7ee2a0, 0x07160c),
            "warn" => (0xffc46b, 0x1a1204),
            _ => (MUTED, 0x101010),
        };
        let lead: AnyElement = match kind {
            "ok" => icon("check", 12., c(fg)).into_any_element(),
            "warn" => icon("info", 12., c(fg)).into_any_element(),
            _ => div().size(px(8.)).bg(c(fg)).into_any_element(),
        };
        div().flex().items_center().gap(px(6.)).h(px(22.)).px(px(8.)).rounded(px(6.)).bg(c(bg)).text_color(c(fg)).font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(11.)).child(lead).child(text)
    }

    fn mk_icon_url(p: &Value) -> String {
        let custom = s(&p["customIcon"]);
        if !custom.is_empty() {
            return custom;
        }
        p["blocks"].as_array().and_then(|l| l.first()).or_else(|| p["items"].as_array().and_then(|l| l.first())).map(|x| s(&x["texture"])).unwrap_or_default()
    }

    fn mk_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut groups = div().flex().flex_col().gap(px(2.));
        let cur = self.v.create.cur.clone();
        for p in self.v.create.projects.clone().unwrap_or_default() {
            let id = s(&p["id"]);
            let on = Some(&id) == cur.as_ref();
            let open = id.clone();
            groups = groups.child(
                div()
                    .id(SharedString::from(format!("prj-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if on { c(LINE2) } else { ca(0x00000000) })
                    .when(on, |d| d.bg(c(0x101010)))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x0c0c0c)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_project(open.clone(), cx)))
                    .child(div().size(px(36.)).rounded(px(8.)).overflow_hidden().bg(c(0x0d0d0d)).child(tex(&Self::mk_icon_url(&p), 36., 8.)))
                    .child(div().flex().flex_col().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).truncate().child(s(&p["name"]))).child(mono(format!("{} · v{}", s(&p["modId"]), s(&p["version"]))).text_color(c(DIM)).truncate())),
            );
        }
        if self.v.create.projects.as_ref().is_some_and(|l| l.is_empty()) {
            groups = groups.child(div().m(px(8.)).text_size(px(13.)).text_color(c(DIM)).child("No mods yet. Make your first one with +."));
        }
        div()
            .w(px(if f32::from(window.viewport_size().width) <= 1100. { 270. } else { 300. }))
            .when(f32::from(window.viewport_size().width) <= 760., |d| d.w_full().border_r_0())
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(16.))
            .border_r_1()
            .border_color(c(0x111111))
            .child(div().flex().items_center().gap(px(8.)).mb(px(6.)).child(pixel_text("Mod Maker", 24.).mr_auto()).child(sq_btn("mk-new", "plus", SqKind::Primary).on_click(cx.listener(|this, _, window, cx| this.new_project(window, cx)))))
            .child(div().flex().items_center().gap(px(8.)).mx(px(4.)).mb(px(10.)).px(px(10.)).py(px(8.)).border_1().border_color(c(0x12323a)).rounded(px(10.)).bg(c(0x04151a)).text_color(c(0x5dd2ea)).child(patchwork_tint(14., 0x5dd2ea)).child(div().text_size(px(12.)).child("Builds Patchwork mods for Breakmine")))
            .child(div().m(px(8.)).child(Btn::new("mk-open-file", "Open project").sm().icon("upload").on_click(cx.listener(|this, _, _, cx| this.import_project_file(false, cx)))))
            .child(groups)
    }

    fn text_field(&self, key: &'static str, label: &str, hint: Option<AnyElement>, mono_font: bool, window: &Window, cx: &gpui::App) -> gpui::Div {
        let input = &self.v.create.inputs[key];
        let rows = FIELDS.iter().find(|f| f.0 == key).map(|f| f.1).unwrap_or(0);
        let mut b = input_box(input, window, cx, mono_font);
        if rows > 0 {
            b = b.h_auto().min_h(px(42.)).items_start().py(px(10.)).text_size(px(13.)).line_height(px(19.5));
        }
        field(label.to_string(), b).children(hint.map(|h| div().text_size(px(11.)).font_weight(FontWeight::NORMAL).text_color(c(DIM2)).child(h)))
    }

    fn details_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mod_ok = g::valid_mod_id(&s(&p["modId"]));
        let custom = !s(&p["customIcon"]).is_empty();
        let pic = div()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .id("mk-pic")
                    .group("mk-pic")
                    .relative()
                    .size(px(72.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(LINE2))
                    .overflow_hidden()
                    .bg(c(0x0d0d0d))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.pick_mod_icon(cx)))
                    .child(tex(&Self::mk_icon_url(p), 72., 4.))
                    .child(div().absolute().right(px(4.)).bottom(px(4.)).size(px(22.)).flex().items_center().justify_center().rounded(px(4.)).bg(ca(0x000000bf)).border_1().border_color(c(LINE2)).opacity(0.).group_hover("mk-pic", |s| s.opacity(1.)).child(icon("upload", 12., c(0xffffff)))),
            )
            .child(if custom {
                link("mk-icon-reset", "Reset image").text_size(px(12.)).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(p) = this.mk_mut() {
                        if let Some(o) = p.as_object_mut() {
                            o.remove("customIcon");
                        }
                    }
                    this.queue_save(cx);
                    cx.notify();
                })).into_any_element()
            } else {
                div().text_size(px(12.)).text_color(c(DIM2)).child("Mod image").into_any_element()
            });
        let half = |el: gpui::Div| el.flex_1().min_w_0();
        let fields = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().flex().gap(px(12.)).child(half(self.text_field("name", "Name", None, false, window, cx))).child(half(self.text_field("modId", "Mod ID", if mod_ok { None } else { Some(div().text_color(c(DANGER)).child("2–32 lowercase letters, numbers or _").into_any_element()) }, false, window, cx))))
            .child(div().flex().gap(px(12.)).child(half(self.text_field("author", "Author", None, false, window, cx))).child(half(self.text_field("version", "Version", None, false, window, cx))).child(half(self.text_field("license", "License", None, false, window, cx))))
            .child(self.text_field("description", "Description", None, false, window, cx));
        let counts = [("block", "Blocks", "blocks"), ("item", "Items", "items"), ("recipe", "Recipes", "recipes"), ("code", "Commands", "commands")];
        let mut stats = div().flex().gap(px(8.));
        for (ic, label, tab) in counts {
            let n = p[tab].as_array().map(|l| l.len()).unwrap_or(0);
            stats = stats.child(
                div()
                    .id(SharedString::from(format!("count-{tab}")))
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(10.))
                    .border_1()
                    .border_color(c(LINE2))
                    .rounded(px(10.))
                    .bg(c(FIELD))
                    .cursor_pointer()
                    .hover(|s| s.border_color(c(0x3a3a3a)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.create.tab = tab;
                        cx.notify();
                    }))
                    .child(icon(ic, 16., c(MUTED)))
                    .child(pixel_text(n.to_string(), 16.).text_color(c(FG)))
                    .child(div().text_size(px(13.)).text_color(c(DIM)).child(label)),
            );
        }
        let mut col = div().flex().flex_col().gap(px(16.)).child(card().flex_none().flex_row().items_start().gap(px(18.)).child(pic).child(fields)).child(stats);
        if !self.v.create.errors.is_empty() {
            let mut e = card().gap(px(6.)).border_color(c(0x3a2a08)).bg(c(0x120c02)).text_color(c(0xffc46b)).child(div().flex().items_center().gap(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(icon("info", 12., c(0xffc46b))).child("Fix before testing"));
            for err in &self.v.create.errors {
                e = e.child(div().text_size(px(13.)).child(err.clone()));
            }
            col = col.child(e);
        }
        col.into_any_element()
    }

    fn list_col(&mut self, kind: &'static str, label: &'static str, items: Vec<(AnyElement, String)>, cx: &mut Context<Self>) -> impl IntoElement {
        let sel = self.sel_of(kind);
        let mut col = div().w(px(210.)).flex_none().flex().flex_col().gap(px(4.));
        let n = items.len();
        for (i, (lead, name)) in items.into_iter().enumerate() {
            let on = sel.min(n.saturating_sub(1)) == i;
            col = col.child(
                div()
                    .id(SharedString::from(format!("row-{kind}-{i}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(44.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if on { c(LINE2) } else { ca(0x00000000) })
                    .when(on, |d| d.bg(c(0x101010)))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x0c0c0c)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.create.sel.insert(kind, i);
                        this.v.create.picker = None;
                        cx.notify();
                    }))
                    .child(lead)
                    .child(div().truncate().child(name)),
            );
        }
        col.child(
            div()
                .id(SharedString::from(format!("add-{kind}")))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .h(px(44.))
                .mt(px(4.))
                .rounded(px(10.))
                .border_1()
                .border_dashed()
                .border_color(c(LINE2))
                .text_size(px(13.))
                .text_color(c(MUTED))
                .cursor_pointer()
                .hover(|s| s.border_color(c(0x4a4a4a)).text_color(c(FG)))
                .on_click(cx.listener(move |this, _, _, cx| this.add_thing(kind, cx)))
                .child(icon("plus", 16., c(MUTED)))
                .child(label),
        )
    }

    fn code_box(&mut self, p: &Value, kind: &'static str, x: &Value, cx: &mut Context<Self>) -> impl IntoElement {
        let key = format!("{kind}:{}", if !s(&x["key"]).is_empty() { s(&x["key"]) } else if !s(&x["name"]).is_empty() { s(&x["name"]) } else { s(&x["result"]) });
        let open = self.v.create.code_open.contains(&key);
        let k2 = key.clone();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .child(link(SharedString::from(format!("code-{key}")), if open { "Hide generated code" } else { "View generated code" }).text_size(px(12.)).child(icon("code", 12., c(0xd4d4d4))).on_click(cx.listener(move |this, _, _, cx| {
                if !this.v.create.code_open.remove(&k2) {
                    this.v.create.code_open.insert(k2.clone());
                }
                cx.notify();
            })))
            .when(open, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("pre-{key}")))
                        .w_full()
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .px(px(16.))
                        .py(px(14.))
                        .border_1()
                        .border_color(c(LINE))
                        .rounded(px(12.))
                        .bg(c(0x030303))
                        .text_color(c(0xc9d6dc))
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(18.6))
                        .whitespace_nowrap()
                        .children(g::preview(p, kind, x).lines().map(|l| div().h(px(18.6)).child(if l.is_empty() { " ".to_string() } else { l.replace('\t', "    ") })).collect::<Vec<_>>()),
                )
            })
    }

    fn pixel_editor(&mut self, id: String, url: String, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ed = self.v.create.pixel.entry(id.clone()).or_default();
        let (tool, color, mirror) = (ed.tool, ed.color.clone(), ed.mirror);
        let img_now = pixel::decode(&url);
        // The canvas: 16×16 cells, 18px each (288px), with a checkerboard under transparent cells.
        let mut canvas_el = div().relative().size(px(288.));
        for y in 0..16u32 {
            for x in 0..16u32 {
                let p = img_now.get_pixel(x, y).0;
                let checker = if (x + y) % 2 == 0 { 0x1a1a1a } else { 0x121212 };
                let mut cell = div().absolute().left(px(x as f32 * 18.)).top(px(y as f32 * 18.)).size(px(18.)).bg(c(checker));
                if p[3] > 0 {
                    cell = cell.child(div().size_full().bg(gpui::Rgba { r: p[0] as f32 / 255., g: p[1] as f32 / 255., b: p[2] as f32 / 255., a: p[3] as f32 / 255. }));
                }
                canvas_el = canvas_el.child(cell);
            }
        }
        let (down_id, move_id) = (id.clone(), id.clone());
        let stage = div()
            .id(SharedString::from(format!("px-{id}")))
            .p(px(6.))
            .border_1()
            .border_color(c(LINE2))
            .rounded(px(10.))
            .bg(c(0x080808))
            .cursor(gpui::CursorStyle::Crosshair)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| this.paint_cell(&down_id, e.position, true, cx)))
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if e.pressed_button == Some(MouseButton::Left) {
                    this.paint_cell(&move_id, e.position, false, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_stroke(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_stroke(cx)))
            .child(canvas_el.child({
                let cell = self.v.create.pixel_origin.clone();
                let key = id.clone();
                gpui::canvas(move |b, _, _| { cell.borrow_mut().insert(key, b.origin); }, |_, _, _, _| {}).absolute().inset_0()
            }));

        let tool_btn = |t: &'static str, label: &'static str| {
            let tid = id.clone();
            let on = tool == t;
            div()
                .id(SharedString::from(format!("tool-{t}-{id}")))
                .size(px(38.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .border_1()
                .border_color(c(if on { ACCENT } else { LINE2 }))
                .bg(c(if on { ACCENT } else { FIELD }))
                .cursor_pointer()
                .hover(|s| s.border_color(c(0x3a3a3a)))
                .child(icon(t, 16., c(if on { INK } else { MUTED })))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.v.create.pixel.entry(tid.clone()).or_default().tool = t;
                    cx.notify();
                }))
                .tooltip(move |_, cx| cx.new(|_| crate::ui::kit::Tip(label)).into())
        };
        let mut palette = div().flex().flex_wrap().w(px(8. * 24. + 7. * 4.)).gap(px(4.));
        for col in pixel::PALETTE {
            let on = color == col;
            let cid = id.clone();
            let [r, g2, b, a] = pixel::rgba(col);
            let mut sw = div()
                .id(SharedString::from(format!("sw-{col}-{id}")))
                .size(px(24.))
                .border_2()
                .border_color(c(if on { ACCENT } else { 0x000000 }))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.v.create.pixel.entry(cid.clone()).or_default().color = col.to_string();
                    let shown = if col == "#00000000" { String::new() } else { col.to_string() };
                    this.v.create.hex.update(cx, |t, cx| {
                        t.set_text(shown);
                        cx.notify();
                    });
                    cx.notify();
                }));
            sw = if a == 0 { sw.bg(c(0x1a1a1a)).child(div().size_full().flex().items_center().justify_center().child(div().w(px(16.)).h(px(2.)).bg(c(0xe23b3b)))) } else { sw.bg(gpui::Rgba { r: r as f32 / 255., g: g2 as f32 / 255., b: b as f32 / 255., a: 1. }) };
            palette = palette.child(sw);
        }
        let act = |name: &'static str, label: &'static str, ic: &'static str| Btn::new(SharedString::from(format!("{name}-{id}")), label).sm().icon(ic).icon_size(12.);
        let (u, r, m, i, cl) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        let side = div()
            .flex_1()
            .min_w(px(200.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().flex().gap(px(6.)).child(tool_btn("pencil", "Pencil")).child(tool_btn("eraser", "Eraser")).child(tool_btn("bucket", "Fill")).child(tool_btn("picker", "Pick color")))
            .child(palette)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(c(MUTED))
                    .child(icon("paint", 12., c(MUTED)))
                    .child(div().size(px(24.)).border_1().border_color(c(LINE2)).bg(c(if color.len() == 7 { u32::from_str_radix(&color[1..], 16).unwrap_or(0) } else { 0x1a1a1a })))
                    .child(div().w(px(110.)).child(input_box(&self.v.create.hex, window, cx, true).h(px(30.)))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(act("undo", "Undo", "rotate").on_click(cx.listener(move |this, _, _, cx| this.pixel_history(&u, false, cx))))
                    .child(act("redo", "Redo", "rotate").on_click(cx.listener(move |this, _, _, cx| this.pixel_history(&r, true, cx))))
                    .child({
                        let b = act("mirror", "Mirror", "copy");
                        let b = if mirror { b.dark() } else { b };
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            let e = this.v.create.pixel.entry(m.clone()).or_default();
                            e.mirror = !e.mirror;
                            cx.notify();
                        }))
                    })
                    .child(act("import", "Import", "upload").on_click(cx.listener(move |this, _, _, cx| this.import_texture(i.clone(), cx))))
                    .child(act("clear", "Clear", "trash").danger_text().on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_texture(&cl, true, cx, |img| *img = image::RgbaImage::new(16, 16));
                    }))),
            );
        div().flex().flex_wrap().items_start().gap(px(16.)).child(stage).child(side)
    }

    fn blocks_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let blocks = p["blocks"].as_array().cloned().unwrap_or_default();
        let items: Vec<(AnyElement, String)> = blocks.iter().map(|b| (tex(&s(&b["texture"]), 28., 4.), s(&b["name"]))).collect();
        let list = self.list_col("block", "Add block", items, cx);
        let Some(i) = self.selected_index("block") else {
            return div().flex().items_start().gap(px(16.)).child(list).child(div().flex_1().child(empty_box("No blocks yet."))).into_any_element();
        };
        let b = blocks[i].clone();
        let texture = s(&b["texture"]);
        let png = data_bytes(&texture).unwrap_or_default();
        let reduce = self.setting_bool_any("reduceMotion");
        self.v.create.cube.update(cx, |c, _| {
            c.reduce_motion = reduce;
            c.set(&format!("{}", util_hash(&texture)), &png, &png, &png);
        });
        let key = s(&b["key"]);
        let hint = if g::valid_key(&key) {
            div().flex().child("In game: ").child(mono(format!("{}:{}", s(&p["modId"]), g::game_id(&key)))).into_any_element()
        } else {
            div().text_color(c(DANGER)).child("lowercase letters, numbers, _").into_any_element()
        };
        let unbreakable = b["unbreakable"].as_bool().unwrap_or(false);
        let hardness = b["hardness"].as_f64().unwrap_or(1.5);
        let light = b["light"].as_f64().unwrap_or(0.);
        let sound = if s(&b["sound"]).is_empty() { "stone".to_string() } else { s(&b["sound"]) };
        let sound_id: SharedString = "mk-sound".into();
        let open = self.dd_open.as_ref() == Some(&sound_id);
        let tid = sound_id.clone();
        let sound_dd = Dropdown::new(sound_id, g::SOUNDS.iter().map(|x| DdOpt::new(*x, format!("{}{}", x[..1].to_uppercase(), &x[1..]))).collect(), sound, open)
            .width(170.)
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
            .on_pick(cx.listener(|this, v: &SharedString, _, cx| {
                if let Some(x) = this.selected_mut("block") {
                    x["sound"] = json!(v.to_string());
                }
                this.dd_open = None;
                this.queue_save(cx);
                cx.notify();
            }));
        let editor = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .child(div().size(px(112.)).flex_none().child(self.v.create.cube.clone()))
                    .child(div().flex_1().flex().flex_col().gap(px(10.)).child(self.text_field("x.name", "Name", None, false, window, cx)).child(self.text_field("x.key", "ID", Some(hint), false, window, cx))),
            )
            .child(card().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Texture")).child(self.pixel_editor(format!("block:{i}"), texture.clone(), window, cx)))
            .child(
                card()
                    .gap(px(12.))
                    .child(div().flex().items_center().gap(px(12.)).child(div().flex_1().flex().gap(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Hardness").child(mono(if unbreakable { "∞".to_string() } else { format!("{hardness:.1}") }).text_color(c(DIM)))).child(div().text_size(px(12.)).text_color(c(DIM)).child("Unbreakable")).child(self.flag_switch("unbreakable", unbreakable, cx)))
                    .child(self.slider("hardness", hardness, 0., 10., 0.5, unbreakable, cx))
                    .child(div().flex().items_center().gap(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Light level").child(mono(format!("{}", light as i64)).text_color(c(DIM))))
                    .child(self.slider("light", light, 0., 15., 1., false, cx))
                    .child(div().flex().flex_wrap().items_center().gap(px(12.)).child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Sound")).child(sound_dd))
                    .child(div().flex().items_center().gap(px(12.)).child(div().flex_1().flex().items_end().gap(px(6.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("See-through")).child(div().text_size(px(12.)).text_color(c(DIM)).child("like glass or leaves"))).child(self.flag_switch("transparent", b["transparent"].as_bool().unwrap_or(false), cx))),
            )
            .child(div().child(Btn::new("del-block", "Delete block").sm().danger_text().icon("trash").icon_size(12.).on_click(cx.listener(|this, _, window, cx| this.delete_thing("block", window, cx)))))
            .child(self.code_box(p, "block", &b, cx));
        div().flex().items_start().gap(px(16.)).child(list).child(editor).into_any_element()
    }

    fn setting_bool_any(&self, key: &str) -> bool {
        self.store.pointer(&format!("/settings/{key}")).and_then(Value::as_bool).unwrap_or(false)
    }

    fn flag_switch(&mut self, flag: &'static str, on: bool, cx: &mut Context<Self>) -> impl IntoElement {
        switch(SharedString::from(format!("flag-{flag}")), on).on_click(cx.listener(move |this, _, _, cx| {
            let kind = this.tab_kind();
            if let Some(x) = this.selected_mut(kind) {
                x[flag] = json!(!on);
            }
            this.queue_save(cx);
            cx.notify();
        }))
    }

    /// `input type=range`: a track with a draggable thumb; values snap to `step`.
    fn slider(&mut self, key: &'static str, value: f64, min: f64, max: f64, step: f64, disabled: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let frac = ((value - min) / (max - min)).clamp(0., 1.) as f32;
        let set = move |this: &mut Launcher, x: f32, w: f32, cx: &mut Context<Launcher>| {
            let t = (x / w.max(1.)).clamp(0., 1.) as f64;
            let v = ((min + t * (max - min)) / step).round() * step;
            let kind = this.tab_kind();
            if let Some(it) = this.selected_mut(kind) {
                it[key] = json!(v);
            }
            this.queue_save(cx);
            cx.notify();
        };
        let bounds: Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>> = Rc::new(std::cell::Cell::new(None));
        let (b1, b2, b3) = (bounds.clone(), bounds.clone(), bounds.clone());
        div()
            .id(SharedString::from(format!("range-{key}")))
            .relative()
            .w_full()
            .h(px(20.))
            .flex()
            .items_center()
            .when(disabled, |d| d.opacity(0.45))
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        if let Some(b) = b1.get() {
                            set(this, f32::from(e.position.x - b.origin.x), f32::from(b.size.width), cx);
                        }
                    }))
                    .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                        if e.pressed_button == Some(MouseButton::Left) {
                            if let Some(b) = b2.get() {
                                set(this, f32::from(e.position.x - b.origin.x), f32::from(b.size.width), cx);
                            }
                        }
                    }))
            })
            .child(gpui::canvas(move |b, _, _| b3.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
            .child(div().w_full().h(px(4.)).rounded(px(2.)).bg(c(0x262626)).child(div().h_full().w(gpui::relative(frac)).rounded(px(2.)).bg(c(ACCENT))))
            .child(div().absolute().top(px(3.)).left(gpui::relative(frac)).ml(px(-7.)).size(px(14.)).bg(c(ACCENT)))
    }

    fn items_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let items_v = p["items"].as_array().cloned().unwrap_or_default();
        let rows: Vec<(AnyElement, String)> = items_v.iter().map(|b| (tex(&s(&b["texture"]), 28., 4.), s(&b["name"]))).collect();
        let list = self.list_col("item", "Add item", rows, cx);
        let Some(i) = self.selected_index("item") else {
            return div().flex().items_start().gap(px(16.)).child(list).child(div().flex_1().child(empty_box("No items yet. Items are things you hold, like ingots or gems."))).into_any_element();
        };
        let it = items_v[i].clone();
        let texture = s(&it["texture"]);
        let key = s(&it["key"]);
        let hint = if g::valid_key(&key) {
            div().flex().child("In game: ").child(mono(format!("{}:{}", s(&p["modId"]), g::game_id(&key)))).into_any_element()
        } else {
            div().text_color(c(DANGER)).child("lowercase letters, numbers, _").into_any_element()
        };
        let float: AnyElement = match data_bytes(&texture).and_then(|b| assets::pixel_bytes(&format!("tex:{}", util_hash(&texture)), &b, 8)) {
            Some(img_) => {
                let image = img(img_).size(px(64.));
                if crate::ui::motion::reduced() { image.into_any_element() } else {
                    image.with_animation("bob", gpui::Animation::new(Duration::from_millis(3200)).repeat(), |el, t| el.mt(px(-6. * (t * std::f32::consts::PI).sin()))).into_any_element()
                }
            },
            None => div().into_any_element(),
        };
        let editor = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().flex().items_center().gap(px(20.)).child(div().size(px(112.)).flex_none().flex().items_center().justify_center().child(float)).child(div().flex_1().flex().flex_col().gap(px(10.)).child(self.text_field("x.name", "Name", None, false, window, cx)).child(self.text_field("x.key", "ID", Some(hint), false, window, cx))))
            .child(card().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Texture")).child(self.pixel_editor(format!("item:{i}"), texture, window, cx)))
            .child(div().child(Btn::new("del-item", "Delete item").sm().danger_text().icon("trash").icon_size(12.).on_click(cx.listener(|this, _, window, cx| this.delete_thing("item", window, cx)))))
            .child(self.code_box(p, "item", &it, cx));
        div().flex().items_start().gap(px(16.)).child(list).child(editor).into_any_element()
    }

    fn recipes_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let things: Vec<(String, String, String, &str)> = p["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| (s(&b["key"]), s(&b["name"]), s(&b["texture"]), "Block"))
            .chain(p["items"].as_array().into_iter().flatten().map(|b| (s(&b["key"]), s(&b["name"]), s(&b["texture"]), "Item")))
            .collect();
        let recipes = p["recipes"].as_array().cloned().unwrap_or_default();
        let rows: Vec<(AnyElement, String)> = recipes
            .iter()
            .map(|r| {
                let o = things.iter().find(|t| t.0 == s(&r["result"]));
                let count = r["count"].as_u64().unwrap_or(1);
                (tex(o.map(|t| t.2.as_str()).unwrap_or(""), 28., 4.), format!("{}{}", o.map(|t| t.1.clone()).unwrap_or(s(&r["result"])), if count > 1 { format!(" ×{count}") } else { String::new() }))
            })
            .collect();
        let list: AnyElement = if things.is_empty() { div().w(px(210.)).into_any_element() } else { self.list_col("recipe", "Add recipe", rows, cx).into_any_element() };
        let Some(i) = self.selected_index("recipe") else {
            let msg = if things.is_empty() { "Add a block or item first, then give it a recipe." } else { "No recipes yet. Add one to make your blocks and items craftable." };
            return div().flex().items_start().gap(px(16.)).child(list).child(div().flex_1().child(empty_box(msg))).into_any_element();
        };
        let r = recipes[i].clone();
        let out = things.iter().find(|t| t.0 == s(&r["result"])).cloned();
        let count = r["count"].as_u64().unwrap_or(1);
        let res_id: SharedString = "mk-result".into();
        let open = self.dd_open.as_ref() == Some(&res_id);
        let tid = res_id.clone();
        let opts: Vec<DdOpt> = things
            .iter()
            .map(|t| {
                let mut o = DdOpt::new(t.0.clone(), t.1.clone()).sub(t.3);
                if let Some(i) = data_bytes(&t.2).and_then(|b| assets::pixel_bytes(&format!("tex:{}", util_hash(&t.2)), &b, 8)) {
                    o = o.img(i, true);
                }
                o
            })
            .collect();
        let result_dd = Dropdown::new(res_id, opts, s(&r["result"]), open).width(220.).on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx))).on_pick(cx.listener(|this, v: &SharedString, _, cx| {
            if let Some(x) = this.selected_mut("recipe") {
                x["result"] = json!(v.to_string());
            }
            this.dd_open = None;
            this.queue_save(cx);
            cx.notify();
        }));
        let amount = |delta: i64, cx: &mut Context<Self>| {
            Btn::new(SharedString::from(format!("amt-{delta}")), if delta > 0 { "+" } else { "−" }).sm().on_click(cx.listener(move |this, _, _, cx| {
                if let Some(x) = this.selected_mut("recipe") {
                    let n = (x["count"].as_i64().unwrap_or(1) + delta).clamp(1, 64);
                    x["count"] = json!(n);
                }
                this.queue_save(cx);
                cx.notify();
            }))
        };
        let grid_v: Vec<u32> = r["grid"].as_array().map(|g| g.iter().map(|v| v.as_u64().unwrap_or(0) as u32).collect()).unwrap_or_else(|| vec![0; 9]);
        let picker = self.v.create.picker;
        let mut grid = div().flex().flex_wrap().w(px(76. * 3. + 8.)).gap(px(4.));
        for (ci, v) in grid_v.iter().enumerate() {
            let ing = VANILLA.iter().find(|x| x.0 == *v);
            let on = picker == Some(ci);
            let mut cell = div()
                .id(SharedString::from(format!("cell-{ci}")))
                .size(px(76.))
                .p(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(c(if on { ACCENT } else { 0x1c1c1c }))
                .rounded(px(6.))
                .bg(c(if ing.is_some() { 0x141414 } else { 0x0d0d0d }))
                .overflow_hidden()
                .cursor_pointer()
                .hover(|s| s.border_color(c(0x4a4a4a)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.v.create.picker = if this.v.create.picker == Some(ci) { None } else { Some(ci) };
                    this.v.create.query.update(cx, |t, cx| {
                        t.clear();
                        cx.notify();
                    });
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                    if let Some(x) = this.selected_mut("recipe") {
                        x["grid"][ci] = json!(0);
                    }
                    this.queue_save(cx);
                    cx.notify();
                }));
            if let Some((_, name, is_item)) = ing {
                cell = cell.child(div().flex().flex_col().items_center().gap(px(4.)).text_size(px(10.)).line_height(gpui::relative(1.15)).text_color(c(0xd4d4d4)).child(icon(if *is_item { "item" } else { "block" }, 12., c(0xd4d4d4))).child(div().line_clamp(2).text_center().child(name.to_string())));
            }
            grid = grid.child(cell);
        }
        let craft = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(22.))
            .p(px(18.))
            .rounded(px(14.))
            .bg(c(0x080808))
            .border_1()
            .border_color(c(LINE))
            .child(grid)
            .child(icon("arrow-right", 24., c(DIM)))
            .child(
                div()
                    .w(px(112.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .children(out.as_ref().map(|o| tex(&o.2, 64., 6.)))
                    .child(pixel_text(if count > 1 { format!("×{count}") } else { String::new() }, 14.).min_h(px(14.)))
                    .child(div().text_size(px(13.)).max_w_full().truncate().child(out.as_ref().map(|o| o.1.clone()).unwrap_or("?".into()))),
            );
        let picker_el: AnyElement = if let Some(_) = picker {
            let q = self.v.create.query.read(cx).value().trim().to_lowercase();
            let mut list = div().id("pick-list").flex().flex_wrap().gap(px(4.)).max_h(px(220.)).overflow_y_scroll();
            let mut any = false;
            for (id, name, is_item) in VANILLA.iter().filter(|x| q.is_empty() || x.1.to_lowercase().contains(&q)).take(80) {
                any = true;
                let id = *id;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("ing-{id}")))
                        .w(px(170.))
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(0x151515)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_ingredient(id, cx)))
                        .child(icon(if *is_item { "item" } else { "block" }, 12., c(FG)))
                        .child(div().flex_1().truncate().child(name.to_string()))
                        .child(mono(id.to_string()).text_color(c(DIM2))),
                );
            }
            if !any {
                list = list.child(div().text_size(px(13.)).text_color(c(DIM)).child("Nothing matches."));
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .border_1()
                .border_color(c(LINE2))
                .rounded(px(14.))
                .bg(c(0x080808))
                .child(search_box(&self.v.create.query, window, cx, false))
                .child(list)
                .child(div().flex().gap(px(8.)).child(Btn::new("ing-empty", "Empty slot").sm().on_click(cx.listener(|this, _, _, cx| this.set_ingredient(0, cx)))).child(div().ml_auto().child(Btn::new("pick-close", "Close").sm().on_click(cx.listener(|this, _, _, cx| {
                    this.v.create.picker = None;
                    cx.notify();
                })))))
                .with_animation("picker-in", gpui::Animation::new(Duration::from_millis(160)), |el, t| el.opacity(t))
                .into_any_element()
        } else {
            div().text_size(px(12.)).text_color(c(DIM2)).child("Click a slot to choose an ingredient. Ingredients are the game’s own blocks and items.").into_any_element()
        };
        let editor = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                card()
                    .gap(px(14.))
                    .child(div().flex().flex_wrap().items_center().gap(px(12.)).child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Makes")).child(result_dd).child(div().flex().items_center().gap(px(8.)).text_size(px(12.)).text_color(c(MUTED)).child("Amount").child(amount(-1, cx)).child(mono(count.to_string()).text_size(px(12.))).child(amount(1, cx))))
                    .child(div().flex().items_center().gap(px(12.)).child(div().flex_1().flex().items_end().gap(px(6.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Shapeless")).child(div().text_size(px(12.)).text_color(c(DIM)).child("ingredients can go anywhere"))).child(self.flag_switch("shapeless", r["shapeless"].as_bool().unwrap_or(false), cx)))
                    .child(craft)
                    .child(picker_el),
            )
            .child(div().child(Btn::new("del-recipe", "Delete recipe").sm().danger_text().icon("trash").icon_size(12.).on_click(cx.listener(|this, _, window, cx| this.delete_thing("recipe", window, cx)))))
            .child(self.code_box(p, "recipe", &r, cx));
        div().flex().items_start().gap(px(16.)).child(list).child(editor).into_any_element()
    }

    fn commands_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cmds = p["commands"].as_array().cloned().unwrap_or_default();
        let rows: Vec<(AnyElement, String)> = cmds
            .iter()
            .map(|x| (div().size(px(28.)).flex_none().flex().items_center().justify_center().rounded(px(4.)).bg(c(0x0d0d0d)).child(pixel_text("/", 16.).text_color(c(0x5dd2ea))).into_any_element(), s(&x["name"])))
            .collect();
        let list = self.list_col("command", "Add command", rows, cx);
        let Some(i) = self.selected_index("command") else {
            return div().flex().items_start().gap(px(16.)).child(list).child(div().flex_1().child(empty_box("No commands yet. Commands are things like /hello that reply in chat."))).into_any_element();
        };
        let cmd = cmds[i].clone();
        let editor = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                card()
                    .gap(px(12.))
                    .child(self.text_field("cmd.name", "Command", Some(div().flex().child("Players type ").child(mono(format!("/{}", s(&cmd["name"])))).child(" in chat").into_any_element()), false, window, cx))
                    .child(self.text_field("cmd.description", "Description", Some(div().child("Shown in /help").into_any_element()), false, window, cx))
                    .child(self.text_field("cmd.reply", "Reply in chat", Some(div().child("One message per line. Colour codes like §a work.").into_any_element()), false, window, cx)),
            )
            .child(div().child(Btn::new("del-command", "Delete command").sm().danger_text().icon("trash").icon_size(12.).on_click(cx.listener(|this, _, window, cx| this.delete_thing("command", window, cx)))))
            .child(self.code_box(p, "command", &cmd, cx));
        div().flex().items_start().gap(px(16.)).child(list).child(editor).into_any_element()
    }

    fn code_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut snips = div().flex().flex_wrap().items_center().gap(px(8.)).child(div().flex_1().flex().items_end().gap(px(6.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("ModLoad.js")).child(div().text_size(px(12.)).text_color(c(DIM)).child("· runs when the mod loads")));
        for (i, (label, _)) in SNIPPETS.iter().enumerate() {
            snips = snips.child(Btn::new(SharedString::from(format!("snip-{i}")), *label).sm().icon("plus").icon_size(10.).on_click(cx.listener(move |this, _, _, cx| {
                let snip = SNIPPETS[i].1;
                if let Some(p) = this.mk_mut() {
                    let code = s(&p["code"]);
                    let sep = if !code.is_empty() && !code.ends_with('\n') { "\n" } else { "" };
                    p["code"] = json!(format!("{code}{sep}{snip}\n"));
                }
                this.v.create.bound.clear();
                this.queue_save(cx);
                cx.notify();
            })));
        }
        card()
            .child(snips)
            .child(code_area(&self.v.create.inputs["code"], window, cx))
            .child(div().flex().flex_wrap().text_size(px(12.)).text_color(c(DIM2)).child("You have ").child(mono("world")).child(", ").child(mono("minecraft")).child(" and ").child(mono("modLoader")).child(". Blocks, items, recipes and commands from the other tabs are registered automatically."))
            .into_any_element()
    }

    fn patches_tab(&mut self, p: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let list = p["patches"].as_array().cloned().unwrap_or_default();
        let index = self.v.create.patch.min(list.len().saturating_sub(1));
        let mut col = div().w(px(210.)).flex_none().flex().flex_col().gap(px(4.));
        for (i, x) in list.iter().enumerate() {
            let on = i == index;
            col = col.child(
                div()
                    .id(SharedString::from(format!("patch-{i}")))
                    .flex()
                    .items_center()
                    .h(px(44.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if on { c(LINE2) } else { ca(0x00000000) })
                    .when(on, |d| d.bg(c(0x101010)))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(0x0c0c0c)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.create.patch = i;
                        cx.notify();
                    }))
                    .child(div().truncate().child(if s(&x["file"]).is_empty() { "New patch".to_string() } else { s(&x["file"]) })),
            );
        }
        col = col
            .child(
                div()
                    .id("patch-add")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .h(px(44.))
                    .mt(px(4.))
                    .rounded(px(10.))
                    .border_1()
                    .border_dashed()
                    .border_color(c(LINE2))
                    .text_size(px(13.))
                    .text_color(c(MUTED))
                    .cursor_pointer()
                    .hover(|s| s.border_color(c(0x4a4a4a)).text_color(c(FG)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(p) = this.mk_mut() {
                            if !p["patches"].is_array() {
                                p["patches"] = json!([]);
                            }
                            p["patches"].as_array_mut().unwrap().push(json!({ "file": "", "source": "// Modify the target module exports here.\n" }));
                            let n = p["patches"].as_array().unwrap().len();
                            this.v.create.patch = n - 1;
                        }
                        this.queue_save(cx);
                        cx.notify();
                    }))
                    .child(icon("plus", 16., c(MUTED)))
                    .child("Add patch"),
            )
            .child(Btn::new("jph-import", "Import .jph").sm().icon("upload").on_click(cx.listener(|this, _, _, cx| this.import_project_file(true, cx))))
            .child(Btn::new("jph-export", "Export .jph").sm().icon("down").disabled(list.is_empty()).on_click(cx.listener(|this, _, _, cx| this.export_jph(cx))))
            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child("JPH contains patches only. Export ZIP to include blocks, items, recipes and other mod files."));
        let editor: AnyElement = if list.is_empty() {
            empty_box("Add a patch or import a .jph file to edit game code.").into_any_element()
        } else {
            card()
                .child(self.text_field("patch.file", "Game file path", None, true, window, cx))
                .child(field("Patch JavaScript", code_area(&self.v.create.inputs["patch.source"], window, cx)))
                .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child("Code runs with the target module's exports and api. Return a cleanup function to undo changes when the mod is disabled. Requires a game build with patch support."))
                .child(div().child(Btn::new("patch-del", "Remove patch").sm().danger_text().icon("trash").on_click(cx.listener(|this, _, _, cx| {
                    let i = this.v.create.patch;
                    if let Some(l) = this.mk_mut().and_then(|p| p["patches"].as_array_mut()) {
                        if i < l.len() {
                            l.remove(i);
                        }
                    }
                    this.v.create.patch = i.saturating_sub(1);
                    this.queue_save(cx);
                    cx.notify();
                }))))
                .into_any_element()
        };
        div().flex().items_start().gap(px(16.)).child(col).child(div().flex_1().min_w_0().flex().flex_col().gap(px(14.)).child(editor)).into_any_element()
    }

    fn mk_detail(&mut self, p: Value, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.bind_inputs(cx);
        let n = |k: &str| p[k].as_array().map(|l| l.len().to_string()).unwrap_or_else(|| "0".into());
        let tabs = [("details", "Details", String::new()), ("blocks", "Blocks", n("blocks")), ("items", "Items", n("items")), ("recipes", "Recipes", n("recipes")), ("commands", "Commands", n("commands")), ("code", "Code", String::new()), ("patches", "Patches", n("patches"))];
        let tab = self.v.create.tab;
        let mut tab_row = div().flex().flex_wrap().gap(px(4.)).px(px(22.)).border_b_1().border_color(c(0x111111)).flex_none();
        for (id, label, count) in tabs {
            let on = tab == id;
            tab_row = tab_row.child(
                div()
                    .id(SharedString::from(format!("mtab-{id}")))
                    .relative()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(c(if on { 0xfafafa } else { DIM }))
                    .cursor_pointer()
                    .hover(|s| s.text_color(c(0xfafafa)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.create.tab = id;
                        this.v.create.picker = None;
                        cx.notify();
                    }))
                    .child(label)
                    .child(mono(count).text_color(c(DIM2)))
                    .child(div().absolute().left(px(12.)).right(px(12.)).bottom(px(-1.)).h(px(3.)).when(on, |d| d.bg(c(ACCENT)))),
            );
        }
        let body = match tab {
            "blocks" => self.blocks_tab(&p, window, cx),
            "items" => self.items_tab(&p, window, cx),
            "recipes" => self.recipes_tab(&p, window, cx),
            "commands" => self.commands_tab(&p, window, cx),
            "code" => self.code_tab(window, cx),
            "patches" => self.patches_tab(&p, window, cx),
            _ => self.details_tab(&p, window, cx),
        };
        let inst = self.v.create.inst.clone().and_then(|id| self.instances.iter().find(|i| i.inst.id == id).cloned()).or_else(|| self.current_instance().cloned());
        let inst_dd: Option<AnyElement> = inst.map(|inst| {
            let dd_id: SharedString = "mk-inst".into();
            let open = self.dd_open.as_ref() == Some(&dd_id);
            let tid = dd_id.clone();
            let opts: Vec<DdOpt> = self
                .instances
                .iter()
                .map(|x| {
                    let mut o = DdOpt::new(x.inst.id.clone(), x.inst.name.clone()).sub(self.build_label(&x.inst));
                    if let Some(i) = icon_image(&x.inst) {
                        o = o.img(i, true);
                    }
                    o
                })
                .collect();
            Dropdown::new(dd_id, opts, inst.inst.id.clone(), open)
                .width(200.)
                .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
                .on_pick(cx.listener(|this, v: &SharedString, _, cx| {
                    this.v.create.inst = Some(v.to_string());
                    this.dd_open = None;
                    cx.notify();
                }))
                .into_any_element()
        });
        let busy = self.v.create.busy;
        let test = Btn::new("mk-test", "Test").primary().disabled(busy || self.busy()).on_click(cx.listener(|this, _, _, cx| this.test_project(cx)));
        let test = if busy { test.leading(spinner("mk-test-spin", c(INK))) } else { test.icon("test") };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .pt(px(22.))
                    .px(px(28.))
                    .pb(px(16.))
                    .child(div().debug_selector(|| "project-header-icon".into()).size(px(56.)).flex_none().rounded(px(12.)).border_1().border_color(c(LINE2)).overflow_hidden().bg(c(0x0d0d0d)).child(tex(&Self::mk_icon_url(&p), 56., 12.)))
                    .child(
                        div()
                            .debug_selector(|| "project-header-title".into())
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .min_w_0()
                            .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).truncate().child(if s(&p["name"]).is_empty() { "Untitled mod".to_string() } else { s(&p["name"]) }))
                            .child(div().flex().gap(px(8.)).child(self.status_badge()).child(chip(s(&p["modId"]), true)).child(chip(format!("v{}", s(&p["version"])), true))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .children(inst_dd)
                            .child(test)
                            .child(Btn::new("mk-export", "Export .zip").icon("down").on_click(cx.listener(|this, _, _, cx| this.export_zip(cx))))
                            .child(Btn::new("mk-save", "Save project").icon("down").on_click(cx.listener(|this, _, _, cx| this.export_project_file(cx))))
                            .child(sq_btn("mk-folder", "folder", SqKind::Default).on_click(cx.listener(|this, _, _, cx| {
                                if this.save_now(cx) {
                                    if let Some(id) = this.v.create.cur.clone() {
                                        if let Ok(d) = projects::dir(&crate::launcher_data(), &id) {
                                            cx.open_with_system(&d.join("mod"));
                                        }
                                    }
                                }
                            })))
                            .child(sq_btn("mk-delete", "trash", SqKind::Default).on_click(cx.listener(|this, _, window, cx| this.delete_project(window, cx)))),
                    ),
            )
            .child(tab_row)
            .child(div().flex().flex_col().gap(px(16.)).pt(px(20.)).px(px(28.)).pb(px(28.)).child(body))
    }

    pub fn create_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.v.create.projects.is_none() {
            self.create_mount(cx);
        }
        let side = self.mk_sidebar(window, cx);
        let main: AnyElement = match self.mk().cloned() {
            Some(p) => self.mk_detail(p, window, cx).into_any_element(),
            None => {
                let reduce = self.setting_bool_any("reduceMotion");
                self.v.create.empty_cube.update(cx, |c, _| c.reduce_motion = reduce);
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .max_w(px(420.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(16.))
                            .py(px(40.))
                            .px(px(20.))
                            .child(div().size(px(168.)).child(self.v.create.empty_cube.clone()))
                            .child(pixel_text("Make a mod", 24.))
                            .child(div().text_size(px(13.)).text_color(c(DIM)).line_height(gpui::relative(1.5)).text_center().child("Paint blocks and items, add recipes and commands, then test them in Breakmine with one click. No coding needed, but you can add your own JavaScript too."))
                            .child(Btn::new("mk-new-big", "New mod").primary().big().icon("plus").on_click(cx.listener(|this, _, window, cx| this.new_project(window, cx))))
                            .child(Btn::new("mk-open-big", "Open project").icon("upload").on_click(cx.listener(|this, _, _, cx| this.import_project_file(false, cx)))),
                    )
                    .into_any_element()
            }
        };
        div().flex_1().min_w_0().flex().when(f32::from(window.viewport_size().width) <= 760., |d| d.flex_col()).child(side).child(main)
    }

    // ── actions ──────────────────────────────────────────────────────────

    fn open_project(&mut self, id: String, cx: &mut Context<Self>) {
        if self.v.create.status == "dirty" {
            self.save_now(cx);
        }
        self.v.create.cur = Some(id);
        self.v.create.pixel.clear();
        self.v.create.patch = 0;
        self.v.create.sel.clear();
        self.v.create.picker = None;
        self.v.create.errors = self.mk().map(g::validate).unwrap_or_default();
        self.v.create.bound.clear();
        cx.notify();
    }

    fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let author = self.active_account().map(|a| a.1).unwrap_or_default();
        self.open_modal(
            "New mod",
            None,
            vec![
                FieldSpec::text("name", "Name", "", "Ruby Mod"),
                FieldSpec::text("author", "Author", author, ""),
                FieldSpec::select("template", "Start with", "block", vec![DdOpt::new("block", "Block"), DdOpt::new("glowing", "Glowing block"), DdOpt::new("item", "Item"), DdOpt::new("patch", "Code patch"), DdOpt::new("empty", "Empty project")]),
            ],
            "Create",
            false,
            None,
            Rc::new(|this, v, _, cx| {
                let get = |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, s)| s.clone()).unwrap_or_default();
                let name = get("name").trim().to_string();
                if name.is_empty() {
                    return;
                }
                let mut mod_id = g::to_key(&name);
                if let Some(stripped) = mod_id.strip_suffix("_mod") {
                    mod_id = stripped.to_string();
                }
                if mod_id.is_empty() {
                    mod_id = "my_mod".into();
                }
                if !mod_id.starts_with(|c: char| c.is_ascii_lowercase()) {
                    mod_id = format!("mod_{mod_id}");
                }
                mod_id.truncate(32);
                let now = util::now();
                let seed = (util::now_millis() % 997) as u64;
                let color = pixel::COLORS[(util::now_millis() % pixel::COLORS.len() as u128) as usize];
                let t = pixel::starter(color, seed);
                let template = get("template");
                let author = if get("author").trim().is_empty() { "Me".to_string() } else { get("author").trim().to_string() };
                let mut p = json!({
                    "id": format!("p-{:x}", util::now_millis()),
                    "created": now, "updated": now,
                    "name": name, "modId": mod_id, "author": author, "version": "1.0.0",
                    "description": "", "license": "All rights reserved",
                    "blocks": [{ "key": format!("{mod_id}_block"), "name": format!("{name} Block"), "texture": t, "hardness": 1.5, "light": 0, "sound": "stone", "transparent": false, "unbreakable": false }],
                    "items": [], "recipes": [], "commands": [], "patches": [],
                    "code": format!("minecraft.addMessageToChat(\"§a{} is loaded!\");", name.replace('"', "")),
                });
                match template.as_str() {
                    "glowing" => {
                        p["blocks"][0]["light"] = json!(15);
                        p["blocks"][0]["name"] = json!(format!("{name} Light"));
                    }
                    "item" => {
                        p["items"] = json!([{ "key": format!("{mod_id}_item"), "name": name, "texture": p["blocks"][0]["texture"].clone() }]);
                        p["blocks"] = json!([]);
                    }
                    "empty" | "patch" => {
                        p["blocks"] = json!([]);
                        p["code"] = json!("");
                    }
                    _ => {}
                }
                if template == "patch" {
                    p["patches"] = json!([{ "file": "", "source": "// Modify the target module exports here.\n" }]);
                }
                let id = s(&p["id"]);
                this.v.create.projects.get_or_insert_with(Vec::new).insert(0, p);
                this.v.create.cur = Some(id);
                this.v.create.pixel.clear();
                this.v.create.patch = 0;
                this.v.create.tab = match template.as_str() {
                    "item" => "items",
                    "patch" => "patches",
                    "empty" => "details",
                    _ => "blocks",
                };
                this.v.create.sel.clear();
                this.v.create.bound.clear();
                this.save_now(cx);
            }),
            window,
            cx,
        );
    }

    fn add_thing(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let Some(p) = self.mk_mut() else { return };
        let m = s(&p["modId"]);
        let next = |base: &str, list: &Value| {
            let l = list.as_array().map(|a| a.len()).unwrap_or(0);
            let mut i = l + 1;
            while list.as_array().into_iter().flatten().any(|x| s(&x["key"]) == format!("{m}_{base}_{i}")) {
                i += 1;
            }
            i
        };
        let new_index;
        match kind {
            "block" => {
                let i = next("block", &p["blocks"]);
                let t = pixel::starter(pixel::COLORS[i % pixel::COLORS.len()], (i * 31) as u64);
                p["blocks"].as_array_mut().unwrap().push(json!({ "key": format!("{m}_block_{i}"), "name": format!("Block {i}"), "texture": t, "hardness": 1.5, "light": 0, "sound": "stone", "transparent": false, "unbreakable": false }));
                new_index = p["blocks"].as_array().unwrap().len() - 1;
            }
            "item" => {
                let i = next("item", &p["items"]);
                p["items"].as_array_mut().unwrap().push(json!({ "key": format!("{m}_item_{i}"), "name": format!("Item {i}"), "texture": "" }));
                new_index = p["items"].as_array().unwrap().len() - 1;
            }
            "recipe" => {
                let first = p["blocks"].as_array().and_then(|l| l.first()).or_else(|| p["items"].as_array().and_then(|l| l.first())).map(|x| s(&x["key"]));
                let Some(first) = first else { return };
                p["recipes"].as_array_mut().unwrap().push(json!({ "result": first, "count": 1, "shapeless": false, "grid": [0, 0, 0, 0, 0, 0, 0, 0, 0] }));
                new_index = p["recipes"].as_array().unwrap().len() - 1;
            }
            _ => {
                let n = p["commands"].as_array().map(|l| l.len()).unwrap_or(0) + 1;
                let base = m.split('_').next().unwrap_or("cmd").to_string();
                p["commands"].as_array_mut().unwrap().push(json!({ "name": format!("{base}{n}"), "description": "", "reply": "§aHello!" }));
                new_index = n - 1;
            }
        }
        self.v.create.sel.insert(kind, new_index);
        self.queue_save(cx);
        cx.notify();
    }

    fn delete_thing(&mut self, kind: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected_index(kind) else { return };
        let x = self.mk().map(|p| p[list_key(kind)][i].clone()).unwrap_or(Value::Null);
        let label = [s(&x["name"]), s(&x["result"])].into_iter().find(|v| !v.is_empty()).unwrap_or_else(|| "this".into());
        self.confirm(
            format!("Delete {label}?"),
            if kind == "block" || kind == "item" { "Recipes that make it are deleted too." } else { "This can’t be undone." },
            "Delete",
            Rc::new(move |this, _, cx| {
                let key = s(&x["key"]);
                if let Some(p) = this.mk_mut() {
                    if let Some(l) = p[list_key(kind)].as_array_mut() {
                        if i < l.len() {
                            l.remove(i);
                        }
                    }
                    if !key.is_empty() {
                        if let Some(r) = p["recipes"].as_array_mut() {
                            r.retain(|r| s(&r["result"]) != key);
                        }
                    }
                }
                let cur = this.sel_of(kind);
                this.v.create.sel.insert(kind, cur.saturating_sub(1));
                this.v.create.bound.clear();
                this.queue_save(cx);
            }),
            window,
            cx,
        );
    }

    fn set_ingredient(&mut self, id: u32, cx: &mut Context<Self>) {
        let Some(slot) = self.v.create.picker else { return };
        let mut next = None;
        if let Some(r) = self.selected_mut("recipe") {
            r["grid"][slot] = json!(id);
            let grid: Vec<u64> = r["grid"].as_array().map(|g| g.iter().map(|v| v.as_u64().unwrap_or(0)).collect()).unwrap_or_default();
            next = grid.iter().enumerate().find(|(i, v)| *i > slot && **v == 0).map(|(i, _)| i);
        }
        self.v.create.picker = next;
        self.queue_save(cx);
        cx.notify();
    }

    // ── pixel painting ───────────────────────────────────────────────────

    fn texture_url(&self, id: &str) -> String {
        let (kind, i) = id.split_once(':').unwrap_or(("block", "0"));
        let i: usize = i.parse().unwrap_or(0);
        self.mk().map(|p| s(&p[list_key(kind)][i]["texture"])).unwrap_or_default()
    }

    /// Change a texture; `history` records an undo step first.
    fn edit_texture(&mut self, id: &str, history: bool, cx: &mut Context<Self>, f: impl FnOnce(&mut image::RgbaImage)) {
        let mut img_ = pixel::decode(&self.texture_url(id));
        if history {
            self.v.create.pixel.entry(id.to_string()).or_default().snapshot(&img_);
        }
        f(&mut img_);
        let url = pixel::encode(&img_);
        let (kind, i) = id.split_once(':').unwrap_or(("block", "0"));
        let i: usize = i.parse().unwrap_or(0);
        if let Some(p) = self.mk_mut() {
            p[list_key(kind)][i]["texture"] = json!(url);
        }
        self.queue_save(cx);
        cx.notify();
    }

    fn paint_cell(&mut self, id: &str, pos: gpui::Point<gpui::Pixels>, start: bool, cx: &mut Context<Self>) {
        // The grid's origin: find it from the stage bounds isn't available here, so use the hitbox recorded by the stage.
        let Some(origin) = self.v.create.pixel_origin.borrow().get(id).copied() else { return };
        let x = (f32::from(pos.x - origin.x) / 18.).floor() as i32;
        let y = (f32::from(pos.y - origin.y) / 18.).floor() as i32;
        if !(0..16).contains(&x) || !(0..16).contains(&y) {
            return;
        }
        let mut img_ = pixel::decode(&self.texture_url(id));
        let ed = self.v.create.pixel.entry(id.to_string()).or_default();
        if start {
            ed.snapshot(&img_);
            ed.last = None;
        }
        let changed = ed.paint(&mut img_, x, y);
        let color = ed.color.clone();
        if !changed {
            let shown = if color == "#00000000" { String::new() } else { color };
            self.v.create.hex.update(cx, |t, cx| {
                t.set_text(shown);
                cx.notify();
            });
            cx.notify();
            return;
        }
        let url = pixel::encode(&img_);
        let (kind, i) = id.split_once(':').unwrap_or(("block", "0"));
        let i: usize = i.parse().unwrap_or(0);
        if let Some(p) = self.mk_mut() {
            p[list_key(kind)][i]["texture"] = json!(url);
        }
        cx.notify();
    }

    fn end_stroke(&mut self, cx: &mut Context<Self>) {
        let mut any = false;
        for ed in self.v.create.pixel.values_mut() {
            if ed.last.take().is_some() {
                any = true;
            }
        }
        if any || self.v.create.status == "dirty" {
            self.queue_save(cx);
        }
    }

    fn pixel_history(&mut self, id: &str, redo: bool, cx: &mut Context<Self>) {
        let now = pixel::decode(&self.texture_url(id));
        let ed = self.v.create.pixel.entry(id.to_string()).or_default();
        let (from, to) = if redo { (&mut ed.redo, &mut ed.undo) } else { (&mut ed.undo, &mut ed.redo) };
        let Some(prev) = from.pop() else { return };
        to.push(now);
        self.edit_texture(id, false, cx, |img_| *img_ = prev);
    }

    fn import_texture(&mut self, id: String, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Import texture".into()) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let loaded = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| image::imageops::resize(&i.to_rgba8(), 16, 16, image::imageops::FilterType::Nearest));
            let _ = view.update(cx, |this, cx| match loaded {
                Some(img_) => this.edit_texture(&id, true, cx, |t| *t = img_),
                None => this.error("That file isn’t an image.", cx),
            });
        })
        .detach();
    }

    fn pick_mod_icon(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose a mod image".into()) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            // Any picture → 64×64 PNG, centre-cropped to a square. Pixel art stays crisp.
            let icon = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| {
                let i = i.to_rgba8();
                let side = i.width().min(i.height());
                let crop = image::imageops::crop_imm(&i, (i.width() - side) / 2, (i.height() - side) / 2, side, side).to_image();
                pixel::encode(&image::imageops::resize(&crop, 64, 64, if side > 64 { image::imageops::FilterType::Triangle } else { image::imageops::FilterType::Nearest }))
            });
            let _ = view.update(cx, |this, cx| match icon {
                Some(url) => {
                    if let Some(p) = this.mk_mut() {
                        p["customIcon"] = json!(url);
                    }
                    this.queue_save(cx);
                    cx.notify();
                }
                None => this.error("That file isn’t an image.", cx),
            });
        })
        .detach();
    }

    // ── files ────────────────────────────────────────────────────────────

    fn save_dialog(&mut self, name: String, ext: &'static str, cx: &mut Context<Self>, write: impl FnOnce(std::path::PathBuf) -> Result<(), String> + 'static, done: impl FnOnce(&mut Launcher, std::path::PathBuf, &mut Context<Launcher>) + 'static) {
        let dir = dirs::home_dir().unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(mut path))) = prompt.await else { return };
            if path.extension().is_none() {
                path.set_extension(ext);
            }
            let result = write(path.clone());
            let _ = view.update(cx, |this, cx| match result {
                Ok(_) => done(this, path, cx),
                Err(e) => this.error(e, cx),
            });
        })
        .detach();
    }

    fn export_zip(&mut self, cx: &mut Context<Self>) {
        if !self.save_now(cx) {
            return;
        }
        let Some(p) = self.mk().cloned() else { return };
        let errs = g::validate(&p);
        if let Some(e) = errs.first() {
            return self.error(e.clone(), cx);
        }
        let id = s(&p["id"]);
        self.save_dialog(
            format!("{}-{}.zip", s(&p["modId"]), s(&p["version"])),
            "zip",
            cx,
            move |out| projects::zip_mod(&crate::launcher_data(), &id, &out),
            |this, out, cx| this.info(format!("Exported {}", out.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()), cx),
        );
    }

    fn export_project_file(&mut self, cx: &mut Context<Self>) {
        if !self.save_now(cx) {
            return;
        }
        let Some(p) = self.mk().cloned() else { return };
        let contents = serde_json::to_string_pretty(&json!({ "format": "breakmine-mod-project", "version": 1, "project": p })).unwrap_or_default();
        self.save_dialog(format!("{}.bmproject", s(&p["modId"])), "bmproject", cx, move |out| std::fs::write(out, contents).map_err(|e| e.to_string()), |this, _, cx| this.info("Project saved", cx));
    }

    fn export_jph(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.mk().cloned() else { return };
        if let Some(e) = g::validate(&p).first() {
            return self.error(e.clone(), cx);
        }
        let patches: Vec<Value> = p["patches"].as_array().into_iter().flatten().map(|x| json!({ "file": g::patch_path(&s(&x["file"])), "source": x["source"] })).collect();
        if patches.is_empty() {
            return self.error("Add a patch first.", cx);
        }
        let data = json!({ "format": "jph", "version": 1, "id": p["modId"], "name": p["name"], "author": p["author"], "description": p["description"], "modVersion": p["version"], "patches": patches });
        let contents = serde_json::to_string_pretty(&data).unwrap_or_default();
        self.save_dialog(format!("{}.jph", s(&p["modId"])), "jph", cx, move |out| std::fs::write(out, contents).map_err(|e| e.to_string()), |this, _, cx| this.info("Patch exported", cx));
    }

    /// "Open project" (.bmproject) or "Import .jph" (patches into the open project).
    fn import_project_file(&mut self, jph: bool, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(if jph { "Import .jph".into() } else { "Open project".into() }) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let data = std::fs::metadata(&path)
                .map_err(|e| e.to_string())
                .and_then(|m| if m.len() > 16 * 1024 * 1024 { Err("File is larger than 16 MB.".into()) } else { Ok(()) })
                .and_then(|_| std::fs::read_to_string(&path).map_err(|e| e.to_string()))
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
            let _ = view.update(cx, |this, cx| {
                let result = data.and_then(|d| if jph { this.apply_jph(d) } else { this.apply_project(d) });
                match result {
                    Ok(msg) => {
                        this.v.create.bound.clear();
                        this.save_now(cx);
                        this.info(msg, cx);
                    }
                    Err(e) => this.error(format!("Couldn't open file: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_jph(&mut self, data: Value) -> Result<String, String> {
        if data["format"] != "jph" || data["version"] != 1 || data["patches"].as_array().map_or(true, |l| l.is_empty()) {
            return Err("Not a supported JPH file.".into());
        }
        let entries: Vec<(Option<String>, String)> = data["patches"].as_array().unwrap().iter().map(|x| (g::patch_path(&s(&x["file"])), s(&x["source"]))).collect();
        if entries.iter().any(|(f, src)| f.is_none() || src.trim().is_empty()) {
            return Err("The JPH contains an invalid patch.".into());
        }
        let p = self.mk_mut().ok_or("Open a project first.")?;
        let mut seen: HashSet<String> = p["patches"].as_array().into_iter().flatten().filter_map(|x| g::patch_path(&s(&x["file"]))).collect();
        for (f, _) in &entries {
            let f = f.clone().unwrap();
            if !seen.insert(f.clone()) {
                return Err(format!("A patch for {f} already exists."));
            }
        }
        if !p["patches"].is_array() {
            p["patches"] = json!([]);
        }
        let list = p["patches"].as_array_mut().unwrap();
        let start = list.len();
        for (f, src) in entries {
            list.push(json!({ "file": f, "source": src }));
        }
        self.v.create.patch = start;
        Ok("Patches imported".into())
    }

    fn apply_project(&mut self, data: Value) -> Result<String, String> {
        let mut p = data["project"].clone();
        let lists_ok = ["blocks", "items", "recipes", "commands"].iter().all(|k| p[*k].as_array().is_some_and(|l| l.iter().all(|x| x.is_object())));
        if data["format"] != "breakmine-mod-project" || data["version"] != 1 || !p.is_object() || !p["name"].is_string() || !p["modId"].is_string() || !lists_ok || (!p["patches"].is_null() && !p["patches"].as_array().is_some_and(|l| l.iter().all(|x| x.is_object()))) {
            return Err("Not a supported Mod Maker project.".into());
        }
        let now = util::now();
        p["id"] = json!(format!("p-{:x}-{:x}", util::now_millis(), util::now_millis() % 0xffffff));
        p["created"] = json!(now);
        p["updated"] = json!(now);
        let id = s(&p["id"]);
        self.v.create.projects.get_or_insert_with(Vec::new).insert(0, p);
        self.v.create.cur = Some(id);
        self.v.create.tab = "details";
        self.v.create.patch = 0;
        self.v.create.pixel.clear();
        Ok("Project opened".into())
    }

    fn test_project(&mut self, cx: &mut Context<Self>) {
        if !self.save_now(cx) {
            return;
        }
        let Some(p) = self.mk().cloned() else { return };
        if let Some(e) = g::validate(&p).first() {
            self.v.create.tab = "details";
            return self.error(e.clone(), cx);
        }
        let inst = self.v.create.inst.clone().and_then(|id| self.instances.iter().find(|i| i.inst.id == id).cloned()).or_else(|| self.current_instance().cloned());
        let Some(inst) = inst else { return self.error("Create an instance first.", cx) };
        let src = match projects::dir(&crate::launcher_data(), &s(&p["id"])) {
            Ok(d) => d.join("mod"),
            Err(e) => return self.error(e, cx),
        };
        let mods = std::path::PathBuf::from(&inst.path).join("mods");
        let _ = std::fs::create_dir_all(&mods);
        match instances::install_folder(&mods, &src) {
            Ok(_) => {
                self.info(format!("{} installed in {}. Starting the game…", s(&p["name"]), inst.inst.name), cx);
                self.refresh();
                self.play(inst.inst.id.clone(), cx);
            }
            Err(e) => self.error(e, cx),
        }
    }

    fn delete_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.mk().cloned() else { return };
        let id = s(&p["id"]);
        self.confirm(
            format!("Delete {}?", s(&p["name"])),
            "The project and its textures are deleted from the launcher. Copies installed in instances or exported .zips stay. This can’t be undone.",
            "Delete",
            Rc::new(move |this, _, cx| {
                if let Ok(d) = projects::dir(&crate::launcher_data(), &id) {
                    let _ = std::fs::remove_dir_all(d);
                }
                if let Some(l) = &mut this.v.create.projects {
                    l.retain(|x| s(&x["id"]) != id);
                }
                this.v.create.cur = this.v.create.projects.as_ref().and_then(|l| l.first()).map(|x| s(&x["id"]));
                this.v.create.bound.clear();
                cx.notify();
            }),
            window,
            cx,
        );
    }
}

/// `.code-edit`: a monospace multi-line field.
fn code_area(input: &Entity<TextInput>, window: &Window, cx: &gpui::App) -> impl IntoElement {
    let focused = input.read(cx).focus_handle_ref().is_focused(window);
    div()
        .id("code-area")
        .w_full()
        .max_h(px(420.))
        .overflow_y_scroll()
        .px(px(12.))
        .py(px(10.))
        .border_1()
        .border_color(c(if focused { 0x3a3a3a } else { 0x222222 }))
        .rounded(px(10.))
        .bg(c(0x030303))
        .text_color(c(0xc9d6dc))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(18.6))
        .child(input.clone())
}


#[cfg(test)]
mod header_layout_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn project_icon_and_title_do_not_overlap(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| crate::layout_test_launcher(cx));
        cx.update(|_, app| view.update(app, |launcher, _| {
            launcher.route = crate::app::Route::Create;
            launcher.v.create.projects = Some(vec![serde_json::json!({
                "id": "layout-test", "name": "e", "modId": "test_mod", "version": "1.0",
                "author": "Test", "license": "MIT", "description": "", "blocks": [],
                "items": [], "recipes": [], "commands": [], "patches": []
            })]);
            launcher.v.create.cur = Some("layout-test".into());
        }));
        for width in [480., 600., 980., 1280.] {
            cx.simulate_resize(gpui::size(px(width), px(720.)));
            cx.refresh().unwrap();
            cx.run_until_parked();
            let icon = cx.debug_bounds("project-header-icon").expect("project icon was rendered");
            let title = cx.debug_bounds("project-header-title").expect("project title was rendered");
            assert!(icon.size.width >= px(55.), "icon shrank at {width}px: {icon:?}");
            assert!(title.left() >= icon.right() + px(10.), "title overlaps icon at {width}px: {icon:?}, {title:?}");
        }
    }
}
