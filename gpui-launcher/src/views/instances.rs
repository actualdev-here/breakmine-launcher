//! Instances (src/js/views/instances.js): grouped card grid, and a page per instance.
use super::{icon_image, INSTANCE_ICONS};
use crate::app::{ago, bytes, playtime, plural, FieldSpec};
use crate::instances::{self, FileEntry, ModInfo};
use crate::text_input::{TextEvent, TextInput};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, Launcher};
use gpui::{
    anchored, deferred, div, img, prelude::*, px, AnimationExt, AnyElement, Context, Entity, FontWeight,
    SharedString, Subscription, Window,
};
use std::rc::Rc;

pub struct State {
    pub filter: &'static str,
    pub query: Entity<TextInput>,
    pub notes: Entity<TextInput>,
    pub tab: &'static str,
    pub mods: Option<Vec<ModInfo>>,
    pub exports: Option<Vec<FileEntry>>,
    pub open: bool,
    pub picker: bool,
    /// Open this instance's page on the next visit (Home's gear buttons).
    pub next: Option<String>,
    importing: bool,
    _subs: Vec<Subscription>,
}

impl State {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        let query = cx.new(|cx| TextInput::with_placeholder(cx, "Search"));
        let notes = cx.new(|cx| TextInput::with_placeholder(cx, "Seeds, coordinates, things to remember…"));
        let subs = vec![
            cx.subscribe(&query, |_, _, _: &TextEvent, cx| cx.notify()),
            cx.subscribe(&notes, |this: &mut Launcher, input, ev: &TextEvent, cx| {
                if matches!(ev, TextEvent::Changed) {
                    let text = input.read(cx).value().to_string();
                    if let Some(id) = this.current_instance().map(|i| i.inst.id.clone()) {
                        if let Ok(mut inst) = instances::read(&this.game_dir, &id) {
                            inst.notes = text;
                            let _ = instances::write(&this.game_dir, &inst);
                        }
                    }
                }
            }),
        ];
        Self { filter: "all", query, notes, tab: "overview", mods: None, exports: None, open: false, picker: false, next: None, importing: false, _subs: subs }
    }
}

const FILTERS: [(&str, &str); 4] = [("all", "All"), ("pinned", "Pinned"), ("modded", "Modded"), ("vanilla", "No mods")];

impl Launcher {
    pub fn current_instance(&self) -> Option<&instances::InstanceView> {
        self.instances.iter().find(|i| i.inst.id == self.selected).or(self.instances.first())
    }

    fn is_running(&self, id: &str) -> bool {
        self.game.as_ref().is_some_and(|g| g.instance_id == id)
    }

    pub fn instances_mount(&mut self, cx: &mut Context<Self>) {
        self.refresh_instances_in_background(cx);
        let s = &mut self.v.instances;
        s.open = s.next.is_some();
        s.picker = false;
        if let Some(id) = s.next.take() {
            self.selected = id;
            self.open_instance_state(cx);
        }
        self.load_versions(cx);
    }

    fn open_instance_state(&mut self, cx: &mut Context<Self>) {
        let notes = self.current_instance().map(|i| i.inst.notes.clone()).unwrap_or_default();
        self.v.instances.notes.update(cx, |t, cx| {
            t.set_text(notes);
            cx.notify();
        });
        self.v.instances.tab = "overview";
        self.v.instances.mods = None;
        self.v.instances.exports = None;
    }

    fn load_tab(&mut self) {
        let Some(it) = self.current_instance() else { return };
        let dir = std::path::PathBuf::from(&it.path);
        let inst = it.inst.clone();
        match self.v.instances.tab {
            "mods" => self.v.instances.mods = Some(instances::list_mods(&dir, &inst)),
            "exports" => self.v.instances.exports = Some(instances::list_exports(&dir)),
            _ => {}
        }
    }

    fn select_and_save(&mut self, id: &str) {
        self.selected = id.to_string();
        if let Some(s) = self.store.get_mut("settings").and_then(|s| s.as_object_mut()) {
            s.insert("selectedInstance".into(), id.into());
        }
        self.save_store();
    }

    // ── grid ─────────────────────────────────────────────────────────────

    fn ins_card(&self, n: usize, p: &instances::InstanceView, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let id = p.inst.id.clone();
        let sel = self.current_instance().is_some_and(|c| c.inst.id == id);
        let busy = self.busy();
        let group: SharedString = format!("ins-card-{id}").into();
        let (open_id, play_id) = (id.clone(), id.clone());
        let lift_key: SharedString = format!("instance-hover-{id}").into();
        let lift = crate::ui::motion::lift(&lift_key, 3.);
        let mut art = div().size(px(48.)).flex().items_center().justify_center();
        if let Some(i) = icon_image(&p.inst) {
            art = art.child(img(i).size(px(48.)));
        }
        crate::ui::motion::hoverable(div()
            .id(SharedString::from(format!("card-{id}"))), lift_key)
            .group(group.clone())
            .relative().top(px(lift))
            .w(px(width))
            .flex_none()
            .border_1()
            .border_color(c(if sel { 0x3a3a3a } else { LINE }))
            .rounded(px(16.))
            .bg(c(CARD))
            .hover(|s| s.border_color(c(0x2c2c2c)).shadow(vec![gpui::BoxShadow { color: ca(0x00000073), offset: gpui::point(px(0.), px(10.)), blur_radius: px(24.), spread_radius: px(0.) }]))
            .child(
                div()
                    .id(SharedString::from(format!("open-{id}")))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .p(px(16.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_and_save(&open_id);
                        this.v.instances.open = true;
                        this.v.instances.picker = false;
                        this.open_instance_state(cx);
                        cx.notify();
                    }))
                    .child(art)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .min_w_0()
                            .w_full()
                            .child(div().w(px(width - 34.)).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).truncate().child(p.inst.name.clone()))
                            .child(div().w(px(width - 34.)).font_family(MONO).text_size(px(12.)).text_color(c(DIM)).truncate().child(self.build_label(&p.inst))),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(c(DIM2))
                            .child(format!("{} · {}", if p.mods_count > 0 { plural(p.mods_count, "mod") } else { "No mods".into() }, playtime(p.inst.playtime))),
                    ),
            )
            .when(self.is_running(&id), |d| {
                d.child(div().absolute().left(px(88.)).top(px(20.)).flex().items_center().gap(px(6.)).text_size(px(11.)).text_color(c(ACCENT)).child(sq(SharedString::from(format!("ins-run-{id}")), false, true)).child("Running"))
            })
            .when(p.inst.pinned, |d| d.child(div().absolute().right(px(16.)).bottom(px(16.)).child(icon("pin", 12., c(DIM2)))))
            .when(!busy, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("play-{id}")))
                        .absolute()
                        .right(px(12.))
                        .top(px(12.))
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.))
                        .bg(c(ACCENT))
                        .cursor_pointer()
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .on_click(cx.listener(move |this, _, _, cx| this.play(play_id.clone(), cx)))
                        .child(icon("play", 14., c(INK))),
                )
            })
            .with_animation(
                SharedString::from(format!("card-in-{n}")),
                gpui::Animation::new(std::time::Duration::from_millis(if crate::ui::motion::reduced() { 1 } else { 460 + 40 * n.min(12) as u64 })),
                move |el, t| {
                    let total = 460. + 40. * n.min(12) as f32;
                    let delay = (80. + 40. * n.min(12) as f32) / total;
                    let k = gpui::ease_out_quint()(((t - delay) / (1. - delay)).clamp(0., 1.));
                    el.opacity(k).mt(px(12. * (1. - k)))
                },
            )
    }

    fn instances_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.v.instances.query.read(cx).value().trim().to_lowercase();
        let filter = self.v.instances.filter;
        let matches = |p: &instances::InstanceView| {
            if !q.is_empty() && !p.inst.name.to_lowercase().contains(&q) {
                return false;
            }
            match filter {
                "pinned" => p.inst.pinned,
                "modded" => p.mods_count > 0,
                "vanilla" => p.mods_count == 0,
                _ => true,
            }
        };
        let mut groups: Vec<String> = Vec::new();
        for i in &self.instances {
            let g = if i.inst.group.is_empty() { "Play".to_string() } else { i.inst.group.clone() };
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        // grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap 12
        let avail = f32::from(window.viewport_size().width) - 56.;
        let cols = ((avail + 12.) / 222.).floor().max(1.);
        let card_w = ((avail - 12. * (cols - 1.)) / cols).floor();

        let mut sections = div().flex().flex_col().gap(px(18.));
        let mut any = false;
        let mut n = 0;
        for g in groups {
            let items: Vec<instances::InstanceView> = self.instances.iter().filter(|p| (if p.inst.group.is_empty() { "Play" } else { &p.inst.group }) == g && matches(p)).cloned().collect();
            if items.is_empty() {
                continue;
            }
            any = true;
            let mut grid = div().flex().flex_wrap().gap(px(12.));
            for p in &items {
                grid = grid.child(self.ins_card(n, p, card_w, cx));
                n += 1;
            }
            sections = sections.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(DIM))
                            .child(g.to_uppercase())
                            .child(mono(items.len().to_string()).text_color(c(DIM2))),
                    )
                    .child(grid),
            );
        }
        if !any {
            sections = sections.child(empty_box(if self.instances.is_empty() { "No instances yet. Make one with New instance." } else { "No instances match." }));
        }
        let mut chips = div().flex().flex_wrap().gap(px(4.));
        for (id, label) in FILTERS {
            chips = chips.child(pill(SharedString::from(format!("f-{id}")), label, filter == id, true).on_click(cx.listener(move |this, _, _, cx| {
                this.v.instances.filter = id;
                cx.notify();
            })));
        }
        let importing = self.v.instances.importing;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(18.))
            .pt(px(22.))
            .px(px(28.))
            .pb(px(28.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(pixel_text("Instances", 32.).mr_auto())
                    .child(div().w(px(220.)).child(search_box(&self.v.instances.query, window, cx, false)))
                    .child(Btn::new("ins-import", if importing { "Importing…" } else { "Import" }).icon("down").disabled(importing).on_click(cx.listener(|this, _, _, cx| this.import_instance(cx))))
                    .child(Btn::new("ins-new", "New instance").primary().icon("plus").on_click(cx.listener(|this, _, window, cx| this.new_instance(window, cx)))),
            )
            .child(chips)
            .child(sections)
    }

    // ── detail ───────────────────────────────────────────────────────────

    fn icon_picker(&self, current: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().flex().flex_wrap().gap(px(4.));
        for name in INSTANCE_ICONS {
            let on = name == current;
            let mut cell = div()
                .id(SharedString::from(format!("pick-{name}")))
                .size(px(46.))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(if on { c(ACCENT) } else { ca(0x00000000) })
                .rounded(px(10.))
                .bg(c(if on { 0x161616 } else { 0x0d0d0d }))
                .cursor_pointer()
                .hover(|s| s.bg(c(0x1a1a1a)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.v.instances.picker = false;
                    if let Some(id) = this.current_instance().map(|c| c.inst.id.clone()) {
                        if let Ok(mut inst) = instances::read(&this.game_dir, &id) {
                            inst.icon = name.to_string();
                            if let Err(e) = instances::write(&this.game_dir, &inst) {
                                this.error(e, cx);
                            }
                        }
                    }
                    this.refresh();
                    cx.notify();
                }));
            if let Some(i) = assets::pixel(&format!("assets/icons/{name}.png"), 8) {
                cell = cell.child(img(i).size(px(32.)));
            }
            grid = grid.child(cell);
        }
        div()
            .id("icon-picker")
            .occlude()
            .mt(px(8.))
            .w(px(320.))
            .p(px(10.))
            .border_1()
            .border_color(c(LINE2))
            .rounded(px(14.))
            .bg(c(CARD))
            .shadow(vec![gpui::BoxShadow { color: ca(0x00000099), offset: gpui::point(px(0.), px(16.)), blur_radius: px(40.), spread_radius: px(0.) }])
            .child(grid)
            .with_animation("picker-in", gpui::Animation::new(std::time::Duration::from_millis(160)).with_easing(gpui::ease_out_quint()), |el, t| el.opacity(t))
    }

    fn overview_tab(&self, it: &instances::InstanceView, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let stats = [(playtime(it.inst.playtime), "Played"), (it.mods_count.to_string(), "Mods on"), (it.exports_count.to_string(), "Exports"), (ago(it.inst.last_played), "Last played")];
        let mut row = div().flex().gap(px(10.));
        for (v, l) in stats {
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(14.))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(14.))
                    .bg(c(CARD))
                    .child(pixel_text(v, 16.))
                    .child(div().text_size(px(13.)).text_color(c(DIM)).child(l)),
            );
        }
        let running = self.is_running(&it.inst.id);
        let notes_focused = self.v.instances.notes.read(cx).focus_handle_ref().is_focused(window);
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(row)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(
                        card()
                            .flex_1()
                            .min_w(px(280.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Game data"))
                            .child(div().text_size(px(12.)).text_color(c(DIM)).line_height(gpui::relative(1.5)).child("This instance has its own game storage: worlds, in-game settings, texture packs and sign-in stay separate from your other instances."))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(8.))
                                    .child(Btn::new("ov-folder", "Open folder").sm().icon("folder").on_click(cx.listener(|this, _, _, cx| this.open_instance_folder("", cx))))
                                    .child(Btn::new("ov-reset", "Reset game data…").sm().danger_text().disabled(running).on_click(cx.listener(|this, _, window, cx| this.reset_profile(window, cx)))),
                            ),
                    )
                    .child(
                        card()
                            .flex_1()
                            .min_w(px(280.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Notes"))
                            .child(
                                div()
                                    .min_h(px(84.))
                                    .px(px(12.))
                                    .py(px(10.))
                                    .border_1()
                                    .border_color(c(if notes_focused { 0x3a3a3a } else { 0x222222 }))
                                    .rounded(px(10.))
                                    .bg(c(FIELD))
                                    .text_size(px(13.))
                                    .line_height(px(19.5))
                                    .child(self.v.instances.notes.clone()),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn mods_tab(&self, it: &instances::InstanceView, cx: &mut Context<Self>) -> AnyElement {
        let Some(mods) = &self.v.instances.mods else {
            return div().text_size(px(13.)).text_color(c(DIM)).child("Loading…").into_any_element();
        };
        let on = mods.iter().filter(|m| m.enabled).count();
        let running = self.is_running(&it.inst.id);
        let summary = if mods.is_empty() { "This instance runs without mods.".to_string() } else { format!("{} on{}", plural(on, "mod"), if running { " · changes apply next launch" } else { "" }) };
        let mut list = div().flex().flex_col().border_1().border_color(c(LINE)).rounded(px(14.)).bg(c(CARD)).overflow_hidden();
        let count = mods.len();
        for (i, m) in mods.iter().enumerate() {
            let tile = div().size(px(40.)).flex_none().flex().items_center().justify_center().rounded(px(10.)).bg(c(0x111111)).overflow_hidden().child(if m.icon_path.is_empty() {
                patchwork_tint(18., 0x5dd2ea)
            } else {
                img(std::path::PathBuf::from(&m.icon_path)).size(px(40.)).rounded(px(10.)).object_fit(gpui::ObjectFit::Cover).into_any_element()
            });
            let sub = [m.author.clone(), m.version.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            let (mid, enabled) = (m.id.clone(), m.enabled);
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .when(i + 1 < count, |d| d.border_b_1().border_color(c(0x101010)))
                    .child(tile)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .min_w_0()
                            .flex_1()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(m.name.clone()))
                            .child(mono(if sub.is_empty() { m.id.clone() } else { sub }).text_color(c(DIM)).truncate()),
                    )
                    .child(switch(SharedString::from(format!("mod-sw-{}", m.id)), m.enabled).on_click(cx.listener(move |this, _, _, cx| {
                        let Some(id) = this.current_instance().map(|c| c.inst.id.clone()) else { return };
                        if let Err(e) = instances::set_mod_enabled(&this.game_dir, &id, &mid, !enabled) {
                            this.error(e, cx);
                        }
                        this.refresh();
                        this.load_tab();
                        cx.notify();
                    }))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().text_size(px(13.)).text_color(c(DIM)).child(summary))
                    .child(div().ml_auto().child(Btn::new("mods-dir", "mods/").sm().icon("folder").on_click(cx.listener(|this, _, _, cx| this.open_instance_folder("mods", cx)))))
                    .child(Btn::new("mods-add", "Add mods").sm().trail("arrow-right").on_click(cx.listener(|this, _, _, cx| this.go(crate::app::Route::Mods, cx)))),
            )
            .when(!mods.is_empty(), |d| d.child(list))
            .into_any_element()
    }

    fn exports_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(files) = &self.v.instances.exports else {
            return div().text_size(px(13.)).text_color(c(DIM)).child("Loading…").into_any_element();
        };
        let summary = if files.is_empty() { "Nothing yet. When the game downloads something (a world export, a structure), it’s saved here.".to_string() } else { plural(files.len(), "file") };
        let mut list = div().flex().flex_col().border_1().border_color(c(LINE)).rounded(px(14.)).bg(c(CARD)).overflow_hidden();
        let count = files.len();
        for (i, f) in files.iter().enumerate() {
            let path = f.path.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .when(i + 1 < count, |d| d.border_b_1().border_color(c(0x101010)))
                    .child(div().size(px(40.)).flex_none().flex().items_center().justify_center().rounded(px(10.)).bg(c(0x111111)).child(icon(if f.file.to_lowercase().ends_with(".zip") { "map" } else { "cube" }, 16., c(0xd4d4d4))))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .min_w_0()
                            .flex_1()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).truncate().child(f.file.clone()))
                            .child(mono(format!("{} · {}", bytes(f.size), ago(f.modified))).text_color(c(DIM))),
                    )
                    .child(Btn::new(SharedString::from(format!("reveal-{i}")), "Show in folder").sm().on_click(move |_, _, cx| cx.reveal_path(std::path::Path::new(&path)))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().text_size(px(13.)).text_color(c(DIM)).child(summary))
                    .child(Btn::new("exports-dir", "exports/").sm().icon("folder").on_click(cx.listener(|this, _, _, cx| this.open_instance_folder("exports", cx)))),
            )
            .when(!files.is_empty(), |d| d.child(list).child(div().text_size(px(12.)).text_color(c(DIM2)).child("To load a world export, use Import in the game’s world list.")))
            .into_any_element()
    }

    fn instance_detail(&mut self, item: instances::InstanceView, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = item.inst.id.clone();
        let running = self.is_running(&id);
        let launching = self.game_starting && self.selected == id;
        let picker = self.v.instances.picker;
        let tab = self.v.instances.tab;
        let tabs = [("overview", "Overview", String::new()), ("mods", "Mods", item.mods_count.to_string()), ("exports", "Exports", item.exports_count.to_string())];
        let mut tab_row = div().flex().flex_wrap().gap(px(4.)).px(px(22.)).border_b_1().border_color(c(0x111111)).flex_none();
        for (tid, label, n) in tabs {
            let on = tab == tid;
            tab_row = tab_row.child(
                div()
                    .id(SharedString::from(format!("itab-{tid}")))
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
                        this.v.instances.tab = tid;
                        this.load_tab();
                        cx.notify();
                    }))
                    .child(label)
                    .child(mono(n).text_color(c(DIM2)))
                    .child(div().absolute().left(px(12.)).right(px(12.)).bottom(px(-1.)).h(px(3.)).when(on, |d| d.bg(c(ACCENT)))),
            );
        }
        let body = match tab {
            "mods" => self.mods_tab(&item, cx),
            "exports" => self.exports_tab(cx),
            _ => self.overview_tab(&item, window, cx),
        };
        let play: AnyElement = if running {
            Btn::new("d-stop", "Stop").stop().icon("stop").on_click(cx.listener(|this, _, _, cx| {
                this.stop_game();
                cx.notify();
            }))
            .into_any_element()
        } else if launching {
            Btn::new("d-busy", "").dark().big().disabled(true).leading(spinner("d-spin", c(FG))).into_any_element()
        } else {
            let pid = id.clone();
            Btn::new("d-play", "Play").primary().big().pixel().icon("play").disabled(self.busy()).on_click(cx.listener(move |this, _, _, cx| this.play(pid.clone(), cx))).into_any_element()
        };
        let pinned = item.inst.pinned;
        let close_picker: Handler = handler({
            let view = cx.entity().downgrade();
            move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.v.instances.picker = false;
                    cx.notify();
                });
            }
        });
        let mut big = div()
            .id("ins-big")
            .relative()
            .size(px(72.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.v.instances.picker = !this.v.instances.picker;
                cx.notify();
            }));
        if let Some(i) = icon_image(&item.inst) {
            big = big.child(img(i).size(px(64.)));
        }
        big = big.child(div().absolute().right(px(-6.)).bottom(px(-6.)).size(px(24.)).flex().items_center().justify_center().rounded(px(8.)).bg(c(ACCENT)).child(icon("edit", 10., c(INK))));
        let icon_name = super::icon_of(&item.inst);
        let iconwrap = div().relative().flex_none().child(big).when(picker, |d| {
            d.child(backdrop("pick-back", window, close_picker.clone())).child(div().absolute().top(px(72.)).left_0().child(deferred(anchored().child(self.icon_picker(icon_name, cx))).with_priority(2)))
        });

        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(16.))
            .pt(px(22.))
            .px(px(28.))
            .pb(px(28.))
            .child(
                div()
                    .id("ins-back")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .text_color(c(DIM))
                    .cursor_pointer()
                    .hover(|s| s.text_color(c(0xffffff)))
                    .child(icon("arrow-right@flipx", 12., c(DIM)))
                    .child("All instances")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.v.instances.open = false;
                        this.v.instances.picker = false;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(18.))
                    .p(px(18.))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(18.))
                    .bg(c(CARD))
                    .child(iconwrap)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .min_w_0()
                            .mr_auto()
                            .child(pixel_text(item.inst.name.clone(), 32.).truncate())
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(6.))
                                    .child(chip(self.build_label(&item.inst), true))
                                    .child(chip(if item.inst.group.is_empty() { "Play".to_string() } else { item.inst.group.clone() }, true))
                                    .when(pinned, |d| d.child(chip("Pinned", true))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(play)
                            .child(sq_btn("d-pin", "pin", if pinned { SqKind::BigPinned } else { SqKind::Big }).on_click(cx.listener(|this, _, _, cx| {
                                if let Some(id) = this.current_instance().map(|c| c.inst.id.clone()) {
                                    if let Ok(mut inst) = instances::read(&this.game_dir, &id) {
                                        inst.pinned = !inst.pinned;
                                        let _ = instances::write(&this.game_dir, &inst);
                                    }
                                }
                                this.refresh();
                                cx.notify();
                            })))
                            .child(sq_btn("d-folder", "folder", SqKind::Big).on_click(cx.listener(|this, _, _, cx| this.open_instance_folder("", cx))))
                            .child(sq_btn("d-copy", "copy", SqKind::Big).on_click(cx.listener(|this, _, _, cx| this.duplicate_instance(cx))))
                            .child(sq_btn("d-edit", "edit", SqKind::Big).on_click(cx.listener(|this, _, window, cx| this.edit_instance(window, cx)))),
                    ),
            )
            .child(tab_row)
            .child(body)
    }

    pub fn instances_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.v.instances.open {
            if let Some(item) = self.current_instance().cloned() {
                if self.v.instances.tab != "overview" && ((self.v.instances.tab == "mods" && self.v.instances.mods.is_none()) || (self.v.instances.tab == "exports" && self.v.instances.exports.is_none())) {
                    self.load_tab();
                }
                return self.instance_detail(item, window, cx).into_any_element();
            }
        }
        self.instances_grid(window, cx).into_any_element()
    }

    // ── actions ──────────────────────────────────────────────────────────

    fn open_instance_folder(&mut self, sub: &str, cx: &mut Context<Self>) {
        if let Some(it) = self.current_instance() {
            let mut path = std::path::PathBuf::from(&it.path);
            if !sub.is_empty() {
                path = path.join(sub);
                let _ = std::fs::create_dir_all(&path);
            }
            cx.open_with_system(&path);
        }
    }

    fn reset_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(it) = self.current_instance().cloned() else { return };
        let name = it.inst.name.clone();
        self.confirm(
            format!("Reset {name}’s game data?"),
            "This deletes the worlds, in-game settings, texture packs and sign-in stored for this instance. Mods and exports stay. It can’t be undone.",
            "Reset",
            Rc::new(move |this, _, cx| {
                if this.is_running(&it.inst.id) {
                    return this.error("Close the game first.", cx);
                }
                let d = std::path::PathBuf::from(&it.path).join("profile-gpui");
                let _ = std::fs::remove_dir_all(&d);
                match std::fs::create_dir_all(&d) {
                    Ok(_) => this.info(format!("{name} starts fresh next launch"), cx),
                    Err(e) => this.error(e.to_string(), cx),
                }
            }),
            window,
            cx,
        );
    }

    fn duplicate_instance(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current_instance().map(|c| c.inst.id.clone()) else { return };
        if self.is_running(&id) {
            return self.error("Close the game before duplicating this instance.", cx);
        }
        match instances::duplicate(&self.game_dir, &id) {
            Ok(copy) => {
                self.select_and_save(&copy.id);
                self.refresh();
                self.open_instance_state(cx);
                self.info(format!("Created {}", copy.name), cx);
            }
            Err(e) => self.error(e, cx),
        }
        cx.notify();
    }

    fn new_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_modal(
            "New instance",
            None,
            vec![FieldSpec::text("name", "Name", "", "Survival"), FieldSpec::text("group", "Folder", "Play", "")],
            "Create",
            false,
            None,
            Rc::new(|this, v, _, cx| {
                let name = v[0].1.trim().to_string();
                let group = v[1].1.trim().to_string();
                match instances::create(&this.game_dir, if name.is_empty() { "New instance" } else { &name }, if group.is_empty() { "Play" } else { &group }) {
                    Ok(inst) => {
                        this.select_and_save(&inst.id);
                        this.refresh();
                        this.v.instances.open = true;
                        this.open_instance_state(cx);
                    }
                    Err(e) => this.error(e, cx),
                }
            }),
            window,
            cx,
        );
    }

    fn edit_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(it) = self.current_instance().cloned() else { return };
        let dev = self.store.pointer("/settings/showDevBuilds").and_then(|v| v.as_bool()).unwrap_or(false);
        let default_build = self.store.pointer("/settings/build").and_then(|v| v.as_str()).unwrap_or("online");
        let label = |b: &str| match b {
            "offline" => "Offline",
            "local" => "Local dev server",
            "custom" => "Custom URL",
            _ => "Online",
        };
        let mut builds = vec![DdOpt::new("default", format!("Launcher default ({})", label(default_build))), DdOpt::new("online", "Online"), DdOpt::new("offline", "Offline")];
        if dev {
            builds.push(DdOpt::new("local", "Local dev server"));
            builds.push(DdOpt::new("custom", "Custom URL"));
        }
        let data = crate::launcher_data();
        let mut versions = vec![DdOpt::new("", "Newest (updates automatically)")];
        for v in self.v.versions.clone().unwrap_or_default() {
            let done = if crate::versions::installed(&data, &v.version) { " · downloaded" } else { "" };
            versions.push(DdOpt::new(v.version.clone(), format!("{} · {}{done}", v.version, v.date.get(..10).unwrap_or(""))));
        }
        if !it.inst.game_version.is_empty() && !versions.iter().any(|o| o.value.as_ref() == it.inst.game_version) {
            versions.push(DdOpt::new(it.inst.game_version.clone(), it.inst.game_version.clone()));
        }
        let id = it.inst.id.clone();
        let name = it.inst.name.clone();
        let del_id = id.clone();
        let extra: crate::app::ModalExtra = Rc::new(move |this, window, cx| {
            let id = del_id.clone();
            let name = name.clone();
            this.confirm(
                format!("Delete {name}?"),
                "This permanently deletes the instance folder: its worlds, settings, mods and exports. It can’t be undone.",
                "Delete",
                Rc::new(move |this, _, cx| {
                    if this.is_running(&id) {
                        return this.error("Close the game before deleting this instance.", cx);
                    }
                    match instances::dir(&this.game_dir, &id).and_then(|d| std::fs::remove_dir_all(d).map_err(|e| e.to_string())) {
                        Ok(_) => {
                            this.info(format!("Deleted {name}"), cx);
                            this.v.instances.open = false;
                            this.refresh();
                        }
                        Err(e) => this.error(e, cx),
                    }
                }),
                window,
                cx,
            );
        });
        self.open_modal(
            "Edit instance",
            None,
            vec![
                FieldSpec::text("name", "Name", it.inst.name.clone(), ""),
                FieldSpec::text("group", "Folder", it.inst.group.clone(), "Play"),
                FieldSpec::select("build", "Game build", if it.inst.build.is_empty() { "default".to_string() } else { it.inst.build.clone() }, builds),
                FieldSpec::select("gameVersion", "Game version (picking an older one switches to the Offline build)", it.inst.game_version.clone(), versions),
            ],
            "Save",
            false,
            Some(("Delete instance".into(), extra)),
            Rc::new(move |this, v, _, cx| {
                let get = |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, s)| s.clone()).unwrap_or_default();
                let name = get("name").trim().to_string();
                if name.is_empty() {
                    return this.error("The name can’t be empty.", cx);
                }
                let result = instances::read(&this.game_dir, &id).and_then(|mut inst| {
                    inst.name = name;
                    inst.group = if get("group").trim().is_empty() { "Play".into() } else { get("group").trim().to_string() };
                    let version = get("gameVersion");
                    inst.build = if version.is_empty() { get("build") } else { "offline".into() };
                    inst.game_version = version;
                    instances::write(&this.game_dir, &inst)
                });
                if let Err(e) = result {
                    this.error(e, cx);
                }
                this.refresh();
            }),
            window,
            cx,
        );
    }

    fn import_instance(&mut self, cx: &mut Context<Self>) {
        if self.v.instances.importing {
            return;
        }
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose an instance folder (close the game first)".into()) });
        let game_dir = self.game_dir.clone();
        let executor = cx.background_executor().clone();
        self.v.instances.importing = true;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else {
                let _ = view.update(cx, |this, cx| {
                    this.v.instances.importing = false;
                    cx.notify();
                });
                return;
            };
            let result = executor.spawn(async move { instances::import_instance(&game_dir, &path) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.instances.importing = false;
                match result {
                    Ok(inst) => {
                        this.select_and_save(&inst.id);
                        this.refresh();
                        this.info(format!("Imported {}", inst.name), cx);
                    }
                    Err(e) => this.error(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
