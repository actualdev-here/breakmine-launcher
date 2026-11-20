//! Servers (src/js/views/servers.js): saved servers with live status, and local server hosting.
use super::scene_of;
use crate::app::FieldSpec;
use crate::host::{hosted, share, shim::AppHandle};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{util, Launcher, ServerCheck};
use gpui::{div, img, prelude::*, px, AnyElement, ClipboardItem, Context, FontWeight, ImageFormat, SharedString, Window};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, rc::Rc, sync::Arc, time::{Duration, Instant}};

pub struct State {
    tab: &'static str,
    sel: Option<String>,
    pub app: AppHandle,
    pub share: Arc<share::ShareState>,
    hosted: Vec<hosted::HostedServer>,
    status: Option<hosted::Status>,
    sharing: Option<share::ShareStatus>,
    busy: bool,
    installing: bool,
    share_installing: bool,
    log_open: bool,
    polling: bool,
    icons: HashMap<String, Option<Arc<gpui::Image>>>,
    player_skin_attempts: HashMap<String, Instant>,
}

impl State {
    pub fn new(_cx: &mut gpui::Context<Launcher>) -> Self {
        Self {
            tab: "fav",
            sel: None,
            app: AppHandle::new(crate::launcher_data()),
            share: Arc::new(share::ShareState::default()),
            hosted: Vec::new(),
            status: None,
            sharing: None,
            busy: false,
            installing: false,
            share_installing: false,
            log_open: false,
            polling: false,
            icons: HashMap::new(),
            player_skin_attempts: HashMap::new(),
        }
    }
}

fn bars(p: Option<u64>) -> usize {
    match p {
        None => 0,
        Some(p) if p < 60 => 4,
        Some(p) if p < 120 => 3,
        Some(p) if p < 250 => 2,
        Some(_) => 1,
    }
}

/// 64×64 PNG from any picture, centre-cropped (servers.js `serverIcon`).
fn square_icon(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|_| "Could not read that image.".to_string())?.to_rgba8();
    let s = img.width().min(img.height());
    if s == 0 {
        return Err("That image is empty.".into());
    }
    let crop = image::imageops::crop_imm(&img, (img.width() - s) / 2, (img.height() - s) / 2, s, s).to_image();
    let out = image::imageops::resize(&crop, 64, 64, if s > 64 { image::imageops::FilterType::Triangle } else { image::imageops::FilterType::Nearest });
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(png)
}

impl Launcher {
    fn ping_of(&self, id: &str) -> (Option<u64>, &'static str) {
        match self.server_checks.get(id) {
            Some(ServerCheck::Reachable(s, _)) => (Some(s.latency_ms), ""),
            Some(ServerCheck::Failed) => (None, "No direct reply"),
            _ => (None, "Checking…"),
        }
    }

    fn ping_text(&self, id: &str) -> String {
        match self.ping_of(id) {
            (Some(p), _) => format!("{p} ms"),
            (None, t) => t.to_string(),
        }
    }

    pub fn servers_mount(&mut self, cx: &mut Context<Self>) {
        if !self.server_checked {
            self.refresh_servers(cx);
        } else {
            self.refresh_server_pings(cx);
        }
        self.load_hosted();
        if !self.v.servers.polling {
            self.v.servers.polling = true;
            let timer = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
              loop {
                timer.timer(Duration::from_secs(3)).await;
                let keep = view
                    .update(cx, |this, cx| {
                        if this.route == crate::app::Route::Servers {
                            if this.v.servers.tab == "hosted" {
                                this.load_hosted();
                                cx.notify();
                            }
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
              }
            })
            .detach();
        }
    }

    fn load_hosted(&mut self) {
        let s = &mut self.v.servers;
        s.hosted = hosted::list(&s.app);
        let status = s.app.host().status(&s.app);
        if status.running_id.is_none() {
            s.share.stop();
        }
        s.sharing = Some(s.share.status(&s.app));
        for h in &s.hosted {
            if !s.icons.contains_key(&h.id) {
                let icon = hosted::server_dir(&s.app, &h.id).ok().and_then(|d| std::fs::read(d.join("server_icon.png")).ok()).filter(|b| b.len() <= 64 * 1024 && util::png_size(b) == Some((64, 64))).map(|b| Arc::new(gpui::Image::from_bytes(ImageFormat::Png, b)));
                s.icons.insert(h.id.clone(), icon);
            }
        }
        s.status = Some(status);
    }

    fn server_list_entries(&self) -> Vec<Value> {
        self.store["servers"].as_array().cloned().unwrap_or_default()
    }

    /// The server's own icon if its status reply had one, else a Breakmine scene. `size` 0 fills the parent.
    fn cover_el(&self, id: &str, size: f32, radius: f32) -> AnyElement {
        let icon = match self.server_checks.get(id) {
            Some(ServerCheck::Reachable(status, _)) => status.icon.as_ref().map(|(_, bytes)| bytes.clone()),
            _ => None,
        };
        let image: AnyElement = match (size > 0., icon) {
            (true, Some(bytes)) => crate::assets::rounded_cover_bytes(bytes, radius, 0.5, 0.68).into_any_element(),
            (false, Some(bytes)) => crate::assets::rounded_cover_top_bytes(bytes, radius, 0.5, 0.68).into_any_element(),
            (true, None) => crate::assets::rounded_cover(scene_of(id), radius, 0.5, 0.68).into_any_element(),
            (false, None) => crate::assets::rounded_cover_top(scene_of(id), radius, 0.5, 0.68).into_any_element(),
        };
        let holder = if size > 0. { div().size(px(size)).flex_none() } else { div().size_full() };
        holder.child(image).into_any_element()
    }

    fn signal(&self, id: &str) -> impl IntoElement {
        let b = bars(self.ping_of(id).0);
        let mut d = div().flex().items_end().gap(px(2.));
        for n in 1..=4 {
            d = d.child(div().w(px(4.)).h(px((n * 3 + 2) as f32)).bg(c(if n <= b { ACCENT } else { 0x2a2a2a })));
        }
        d
    }

    /// Player names are optional in server status; a missing list is not an empty server.
    fn server_players(&mut self, id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (names, count) = match self.server_checks.get(id) {
            Some(ServerCheck::Reachable(status, _)) => (status.player_names.clone(), status.players),
            _ => return None,
        };
        if names.is_empty() {
            return match count {
                Some(0) => Some(div().text_size(px(12.)).text_color(c(DIM)).child("No players online").into_any_element()),
                Some(_) => Some(div().text_size(px(12.)).text_color(c(DIM)).child("Player names are unavailable for this server.").into_any_element()),
                None => None,
            };
        }
        const LIMIT: usize = 24;
        let remaining = names.len().saturating_sub(LIMIT);
        let mut players = div().flex().flex_wrap().gap(px(8.));
        for name in names.into_iter().take(LIMIT) {
            // Only valid account names can be looked up through the public skin API.
            let valid = (3..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if valid {
                let missing = !self.remote_skins.contains_key(&name);
                let unavailable = self.remote_skins.get(&name).is_some_and(Option::is_none);
                let interval = if unavailable { 60 } else { 300 };
                let due = self.v.servers.player_skin_attempts.get(&name)
                    .is_some_and(|last| last.elapsed() >= Duration::from_secs(interval));
                if missing || due {
                    self.v.servers.player_skin_attempts.insert(name.clone(), Instant::now());
                    self.fetch_remote_skin(name.clone(), cx);
                } else {
                    self.v.servers.player_skin_attempts.entry(name.clone()).or_insert_with(Instant::now);
                }
            }
            let bytes = self.remote_skins.get(&name).and_then(|skin| skin.as_ref()).cloned().unwrap_or_else(crate::assets::default_skin);
            let key = format!("server-player:{}:{:x}", name, Sha256::digest(&bytes));
            let face = crate::assets::skin_head(&key, &bytes, 8)
                .or_else(|| crate::assets::skin_head("server-player:default", &crate::assets::default_skin(), 8));
            players = players.child(
                div().debug_selector({ let name = name.clone(); move || format!("server-player-{name}").into() })
                    .flex().items_center().gap(px(7.)).min_w_0().max_w_full()
                    .px(px(8.)).py(px(6.)).rounded(px(8.)).bg(c(0x111111))
                    .child(match face {
                        Some(image) => img(image).size(px(24.)).rounded(px(4.)).flex_none().into_any_element(),
                        None => div().size(px(24.)).rounded(px(4.)).bg(c(0x252525)).flex_none().into_any_element(),
                    })
                    .child(div().text_size(px(12.)).text_color(c(FG)).truncate().child(name)),
            );
        }
        if remaining > 0 {
            players = players.child(div().text_size(px(12.)).text_color(c(DIM)).px(px(4.)).py(px(8.)).child(format!("+{remaining} more")));
        }
        Some(div().flex().flex_col().gap(px(8.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(c(MUTED)).child("Online players"))
            .child(players).into_any_element())
    }

    fn saved_servers(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let all = self.server_list_entries();
        let tab = self.v.servers.tab;
        let mut list: Vec<Value> = match tab {
            "own" => all.iter().filter(|s| s["own"].as_bool().unwrap_or(false)).cloned().collect(),
            "recent" => {
                let mut l: Vec<Value> = all.iter().filter(|s| s["lastJoined"].as_u64().unwrap_or(0) > 0).cloned().collect();
                l.sort_by(|a, b| b["lastJoined"].as_u64().cmp(&a["lastJoined"].as_u64()));
                l
            }
            _ => all.iter().filter(|s| !s["own"].as_bool().unwrap_or(false)).cloned().collect(),
        };
        let sel = self.v.servers.sel.clone().and_then(|id| all.iter().find(|s| s["id"].as_str() == Some(&id)).cloned()).or_else(|| list.first().cloned()).or_else(|| all.first().cloned());
        let sel_id = sel.as_ref().and_then(|s| s["id"].as_str()).unwrap_or("").to_string();

        let mut col = div().flex().flex_col().gap(px(8.)).flex_grow().flex_shrink().min_w_0().flex_basis(px(480.));
        // Tauri's .srv-list uses flex: 999 1 480px; extra width goes to the list.
        col.style().flex_grow = Some(999.);
        if list.is_empty() {
            col = col.child(empty_box(if tab == "recent" { "Servers you join show up here." } else { "No servers here yet." }));
        }
        for s in list.drain(..) {
            let id = s["id"].as_str().unwrap_or("").to_string();
            let on = id == sel_id;
            let players = self.player_text(&id).unwrap_or_default();
            let pick = id.clone();
            col = col.child(
                div()
                    .id(SharedString::from(format!("srv-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .p(px(12.))
                    .rounded(px(14.))
                    .bg(c(CARD2))
                    .border_1()
                    .border_color(c(if on { 0xe5e5e5 } else { 0x171717 }))
                    .cursor_pointer()
                    .when(!on, |d| d.hover(|s| s.border_color(c(0x2a2a2a))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.v.servers.sel = Some(pick.clone());
                        cx.notify();
                    }))
                    .child(self.cover_el(&id, 56., 10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(s["name"].as_str().unwrap_or("").to_string()))
                            .child(div().text_size(px(13.)).text_color(c(MUTED)).truncate().child(s["motd"].as_str().unwrap_or("").to_string()))
                            .child(mono(players).text_color(c(DIM)).truncate())
                            .child(mono(format!("{}{}", s["address"].as_str().unwrap_or(""), if s["protocol"].as_str() == Some("json") { " · QuickJSON" } else { "" })).text_color(c(DIM)).truncate()),
                    )
                    .child(div().flex().flex_col().items_end().gap(px(6.)).child(self.signal(&id)).child(mono(self.ping_text(&id)).text_color(c(DIM)))),
            );
        }
        let mut row = div().flex().flex_wrap().items_start().gap(px(20.)).child(col);
        if let Some(s) = sel {
            let id = sel_id.clone();
            let (ping, _) = self.ping_of(&id);
            let players = self.player_text(&id);
            let inst = self.current_instance().map(|i| i.inst.name.clone());
            let can_join = !self.busy() && inst.is_some();
            let motd = s["motd"].as_str().unwrap_or("").to_string();
            let address = s["address"].as_str().unwrap_or("").to_string();
            let (jid, eid, rid) = (id.clone(), id.clone(), id.clone());
            let mut chips = div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(chip_line(div().size(px(6.)).bg(c(if ping.is_some() { ACCENT } else { 0x2a2a2a })).into_any_element(), self.ping_text(&id)));
            if let Some(p) = players {
                chips = chips.child(chip_line(div().into_any_element(), p));
            }
            if let Some(n) = inst {
                chips = chips.child(chip_line(div().into_any_element(), format!("Joins with {n}")));
            }
            let online_players = self.server_players(&id, cx);
            row = row.child(
                div()
                    .flex_grow()
                    .flex_basis(px(340.))
                    .flex_shrink()
                    .min_w_0()
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(18.))
                    .overflow_hidden()
                    .bg(c(CARD))
                    .child(div().h(px(180.)).w_full().overflow_hidden().child(self.cover_el(&id, 0., 17.).into_any_element()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .p(px(18.))
                            .child(div().flex().flex_col().gap(px(4.)).child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child(s["name"].as_str().unwrap_or("").to_string())).child(div().font_family(MONO).text_size(px(12.)).text_color(c(DIM)).child(address.clone())))
                            .when(!motd.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(c(MUTED)).line_height(gpui::relative(1.5)).child(motd)))
                            .child(chips)
                            .children(online_players)
                            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child(format!(
                                "Join opens the game on Multiplayer and connects (you need to be signed in to breakmine.com). Every server here is also added to the in-game list.{}",
                                if matches!(self.server_checks.get(&id), Some(ServerCheck::Failed)) { " No direct reply is normal for servers reached through the game’s proxy." } else { "" }
                            )))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(div().flex_1().child(Btn::new("srv-join", "Join").primary().icon("play").full().disabled(!can_join).on_click(cx.listener(move |this, _, _, cx| this.join_server(&jid, cx)))))
                                    .child(sq_btn("srv-copy", "copy", SqKind::Big).on_click(cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(address.clone()));
                                        this.info("Address copied", cx);
                                    })))
                                    .child(sq_btn("srv-edit", "edit", SqKind::Big).on_click(cx.listener(move |this, _, window, cx| this.edit_server(Some(eid.clone()), window, cx))))
                                    .child(sq_btn("srv-rm", "trash", SqKind::Big).on_click(cx.listener(move |this, _, window, cx| this.remove_server(rid.clone(), window, cx)))),
                            ),
                    ),
            );
        }
        row.into_any_element()
    }

    fn hosted_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.v.servers;
        let status = s.status.as_ref();
        let running_id = status.and_then(|s| s.running_id.clone());
        let ready = status.is_some_and(|s| s.runtime_ready);
        let busy = s.busy;
        let progress = s.app.event("hosted-install-progress");
        let mut top = card()
            .gap(px(8.))
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Local servers"))
            .child(div().text_size(px(13.)).text_color(c(DIM)).line_height(gpui::relative(1.5)).child(
                "Runs on this computer. Closing the launcher window while a server is running minimizes it; stop the server to quit. Choose local network access if friends on your network should join. Server status shares online player names. Changing the seed or world type only affects a new world.",
            ));
        let mut actions = div().flex().flex_wrap().items_center().gap(px(8.)).child(Btn::new("host-create", "Create server").primary().sm().icon("plus").on_click(cx.listener(|this, _, window, cx| this.edit_hosted(None, window, cx))));
        if ready {
            actions = actions
                .child(div().text_size(px(13.)).text_color(c(DIM)).child("Server runtime installed"))
                .child(Btn::new("host-uninstall", "Uninstall runtime").sm().danger_text().disabled(busy || running_id.is_some()).on_click(cx.listener(|this, _, window, cx| this.uninstall_runtime(window, cx))));
        } else {
            actions = actions
                .child(Btn::new("host-setup", if s.installing { "Installing…" } else { "Install server runtime" }).sm().disabled(busy).on_click(cx.listener(|this, _, _, cx| this.install_runtime(cx))))
                .child(div().text_size(px(12.)).text_color(c(DIM2)).child("Downloads Breakmine and dependencies. Requires Git and Node.js 22+."));
        }
        top = top.child(actions);
        if s.installing {
            let phase = progress.as_ref().and_then(|p| p["phase"].as_str()).unwrap_or("Starting setup").to_string();
            let pct = progress.as_ref().and_then(|p| p["percent"].as_u64()).map(|p| p.min(100) as usize);
            let mut blocks = div().flex().gap(px(2.));
            for i in 0..20 {
                blocks = blocks.child(div().flex_1().h(px(6.)).bg(c(if pct.is_some_and(|p| i < (p + 2) / 5) { ACCENT } else { LINE2 })));
            }
            top = top.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().flex().items_center().gap(px(8.)).child(spinner("host-spin", c(FG))).child(div().text_size(px(13.)).child(match pct {
                        Some(p) => format!("{phase} {p}%"),
                        None => format!("{phase}…"),
                    })))
                    .child(blocks),
            );
        }
        let mut col = div().flex().flex_col().gap(px(14.)).child(top);
        let hosted = s.hosted.clone();
        let sharing = s.sharing.as_ref().map(|x| (x.server_id.clone(), x.url.clone()));
        let share_installing = s.share_installing;
        let last_log = status.filter(|st| !st.log.is_empty()).map(|st| (st.last_id.clone(), st.log.clone()));
        let log_open = s.log_open;
        for h in hosted {
            let running = running_id.as_deref() == Some(h.id.as_str());
            let saved_id = format!("host-{}", h.id);
            let players = if running { self.player_text(&saved_id) } else { None };
            let player_list = if running { self.server_players(&saved_id, cx) } else { None };
            let icon_img = self.v.servers.icons.get(&h.id).cloned().flatten();
            let meta = format!(
                "127.0.0.1:{} · {} · {} · {} · {}{}",
                h.port,
                h.world_type,
                h.gamemode,
                if h.allow_lan { "Local network" } else { "This computer" },
                if running { "Running" } else { "Stopped" },
                players.map(|p| format!(" · {p}")).unwrap_or_default()
            );
            let (sid, jid, eid, iid, fid, did, shid) = (h.id.clone(), h.id.clone(), h.id.clone(), h.id.clone(), h.id.clone(), h.id.clone(), h.id.clone());
            let shared_here = sharing.as_ref().is_some_and(|(id, _)| id.as_deref() == Some(h.id.as_str()));
            let mut head = div().flex().flex_wrap().items_center().gap(px(10.));
            if let Some(i) = &icon_img {
                head = head.child(img(i.clone()).size(px(48.)).rounded(px(10.)));
            }
            head = head
                .child(div().flex_1().min_w(px(200.)).flex().flex_col().gap(px(4.)).child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(h.name.clone())).child(mono(meta).text_color(c(DIM))).children(player_list))
                .child({
                    let b = Btn::new(SharedString::from(format!("hs-{}", h.id)), if running { "Stop" } else { "Start" }).sm().disabled(busy || (!running && running_id.is_some()));
                    let b = if running { b } else { b.primary() };
                    b.on_click(cx.listener(move |this, _, _, cx| if running { this.stop_hosted(cx) } else { this.start_hosted(sid.clone(), cx) }))
                })
                .child(Btn::new(SharedString::from(format!("hj-{}", h.id)), "Join").sm().disabled(!running || self.busy()).on_click(cx.listener(move |this, _, _, cx| this.join_hosted(&jid, cx))));
            if running {
                head = head.child(if shared_here {
                    Btn::new(SharedString::from(format!("ss-{}", h.id)), "Stop sharing").sm().on_click(cx.listener(|this, _, _, cx| {
                        this.v.servers.share.stop();
                        this.load_hosted();
                        this.info("Sharing stopped", cx);
                    }))
                } else {
                    Btn::new(SharedString::from(format!("sh-{}", h.id)), "Share online").sm().disabled(busy).on_click(cx.listener(move |this, _, window, cx| this.share_hosted(shid.clone(), window, cx)))
                });
            }
            head = head
                .child(Btn::new(SharedString::from(format!("he-{}", h.id)), "Settings").sm().disabled(running).on_click(cx.listener(move |this, _, window, cx| this.edit_hosted(Some(eid.clone()), window, cx))))
                .child(Btn::new(SharedString::from(format!("hi-{}", h.id)), if icon_img.is_some() { "Change icon" } else { "Set icon" }).sm().icon("upload").disabled(running).on_click(cx.listener(move |this, _, _, cx| this.pick_host_icon(iid.clone(), cx))))
                .child(Btn::new(SharedString::from(format!("hf-{}", h.id)), "Folder").sm().on_click(cx.listener(move |this, _, _, cx| {
                    if let Ok(d) = hosted::server_dir(&this.v.servers.app, &fid) {
                        cx.open_with_system(&d);
                    }
                })))
                .child({
                    let b = sq_btn(SharedString::from(format!("hd-{}", h.id)), "trash", SqKind::Default);
                    if running { b.opacity(0.45) } else { b.on_click(cx.listener(move |this, _, window, cx| this.delete_hosted(did.clone(), window, cx))) }
                });
            let mut card_el = card().child(head);
            if running && shared_here {
                let url = sharing.as_ref().and_then(|(_, u)| u.clone());
                let mut box_ = card().gap(px(8.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(if url.is_some() { "Sharing online" } else { "Connecting tunnel…" }));
                match url {
                    Some(u) => {
                        let host = url::Url::parse(&u).ok().and_then(|x| x.host_str().map(String::from)).unwrap_or(u.clone());
                        let copy = host.clone();
                        box_ = box_
                            .child(div().flex().flex_wrap().items_center().gap(px(8.)).child(mono(host)).child(Btn::new("share-copy", "Copy address").sm().icon("copy").on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                this.info("Server address copied", cx);
                            }))))
                            .child(div().text_size(px(12.)).text_color(c(DIM2)).line_height(gpui::relative(1.5)).child("Anyone with this address can try to join. The temporary address changes when sharing restarts."));
                    }
                    None => box_ = box_.child(div().text_size(px(12.)).text_color(c(DIM2)).child("Waiting for Cloudflare to provide an address.")),
                }
                card_el = card_el.child(box_);
            }
            if running && share_installing {
                let p = self.v.servers.app.event("share-install-progress").and_then(|v| v["percent"].as_u64());
                card_el = card_el.child(div().text_size(px(13.)).text_color(c(DIM)).child(match p {
                    Some(p) => format!("Installing sharing tool {p}%"),
                    None => "Installing sharing tool…".into(),
                }));
            }
            if let Some((Some(last), log)) = &last_log {
                if *last == h.id {
                    card_el = card_el
                        .child(
                            div()
                                .id(SharedString::from(format!("log-{}", h.id)))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(13.))
                                .text_color(c(DIM))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.v.servers.log_open = !this.v.servers.log_open;
                                    cx.notify();
                                }))
                                .child(icon(if log_open { "chevron" } else { "chevron-right" }, 12., c(DIM)))
                                .child("Server log"),
                        )
                        .when(log_open, |d| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!("logbox-{}", h.id)))
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
                                    .child(log.clone()),
                            )
                        });
                }
            }
            col = col.child(card_el);
        }
        col.into_any_element()
    }

    pub fn servers_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.v.servers.tab;
        let mut tabs = seg();
        for (id, label) in [("fav", "Favorites"), ("own", "Your worlds"), ("recent", "Recent"), ("hosted", "Local servers")] {
            tabs = tabs.child(pill(SharedString::from(format!("st-{id}")), label, tab == id, false).on_click(cx.listener(move |this, _, _, cx| {
                this.v.servers.tab = id;
                if id == "hosted" {
                    this.load_hosted();
                }
                cx.notify();
            })));
        }
        let body = if tab == "hosted" { self.hosted_panel(cx) } else { self.saved_servers(cx) };
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
                    .child(pixel_text("Servers", 32.).mr_auto())
                    .child(tabs)
                    .child(Btn::new("srv-refresh", "Refresh").icon("rotate").on_click(cx.listener(|this, _, _, cx| this.refresh_servers(cx))))
                    .child(Btn::new("srv-add", "Add server").primary().icon("plus").on_click(cx.listener(|this, _, window, cx| this.edit_server(None, window, cx)))),
            )
            .child(body)
    }

    // ── saved servers ────────────────────────────────────────────────────

    fn edit_server(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = id.as_ref().and_then(|id| self.server_list_entries().into_iter().find(|s| s["id"].as_str() == Some(id)));
        let get = |k: &str| existing.as_ref().and_then(|s| s[k].as_str()).unwrap_or("").to_string();
        let protocol = if get("protocol") == "json" { "json" } else { "binary" };
        let own = existing.as_ref().and_then(|s| s["own"].as_bool()).unwrap_or(false);
        self.open_modal(
            if existing.is_some() { "Edit server" } else { "Add server" },
            None,
            vec![
                FieldSpec::text("name", "Name", get("name"), "My server"),
                FieldSpec::text("address", "Address", get("address"), "play.example.com or localhost:8080"),
                FieldSpec::area("motd", "Description", get("motd"), ""),
                FieldSpec::select("protocol", "Protocol", protocol, vec![DdOpt::new("binary", "Binary (standard)"), DdOpt::new("json", "QuickJSON")]),
                FieldSpec::select("own", "List under", if own { "own" } else { "fav" }, vec![DdOpt::new("fav", "Favorites"), DdOpt::new("own", "Your worlds")]),
            ],
            if existing.is_some() { "Save" } else { "Add" },
            false,
            None,
            Rc::new(move |this, v, _, cx| {
                let get = |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, s)| s.trim().to_string()).unwrap_or_default();
                if get("address").is_empty() {
                    return this.error("A server needs an address.", cx);
                }
                let name = if get("name").is_empty() { get("address") } else { get("name") };
                if !this.store["servers"].is_array() {
                    this.store["servers"] = json!([]);
                }
                let list = this.store["servers"].as_array_mut().unwrap();
                let data = json!({ "name": name, "address": get("address"), "motd": get("motd"), "protocol": get("protocol"), "own": get("own") == "own" });
                match &id {
                    Some(id) => {
                        if let Some(s) = list.iter_mut().find(|s| s["id"].as_str() == Some(id)) {
                            for (k, v) in data.as_object().unwrap() {
                                s[k] = v.clone();
                            }
                        }
                    }
                    None => {
                        let new_id = format!("s-{:x}", util::now_millis());
                        let mut s = data.clone();
                        s["id"] = json!(new_id.clone());
                        s["lastJoined"] = json!(0);
                        list.push(s);
                        this.v.servers.sel = Some(new_id);
                    }
                }
                this.save_store();
                this.refresh_servers(cx);
            }),
            window,
            cx,
        );
    }

    fn remove_server(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.server_list_entries().into_iter().find(|s| s["id"].as_str() == Some(&id)).and_then(|s| s["name"].as_str().map(String::from)).unwrap_or_default();
        self.confirm(
            format!("Remove {name}?"),
            "It will be taken off your server list.",
            "Remove",
            Rc::new(move |this, _, cx| {
                if let Some(list) = this.store["servers"].as_array_mut() {
                    list.retain(|s| s["id"].as_str() != Some(&id));
                }
                if crate::local_servers::remove(&crate::gpui_servers_path(), &id).is_err() {
                    // Not a GPUI-only server; it was in the copied store.
                }
                this.v.servers.sel = None;
                this.save_store();
                cx.notify();
            }),
            window,
            cx,
        );
    }

    // ── local servers ────────────────────────────────────────────────────

    fn edit_hosted(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = id.as_ref().and_then(|id| self.v.servers.hosted.iter().find(|s| &s.id == id).cloned());
        let owner = existing.as_ref().map(|e| e.owner.clone()).unwrap_or_else(|| self.active_account().map(|a| a.1).unwrap_or_default());
        let e = existing.clone();
        let g = |f: fn(&hosted::HostedServer) -> String, d: &str| e.as_ref().map(f).unwrap_or_else(|| d.to_string());
        self.open_modal(
            if existing.is_some() { "Edit local server" } else { "Create local server" },
            None,
            vec![
                FieldSpec::text("name", "Name", g(|s| s.name.clone(), ""), "My server"),
                FieldSpec::text("port", "Port", g(|s| s.port.to_string(), "6008"), ""),
                FieldSpec::select("worldType", "World type", g(|s| s.world_type.clone(), "flat"), vec![DdOpt::new("flat", "Flat"), DdOpt::new("normal", "Normal"), DdOpt::new("amplified", "Amplified")]),
                FieldSpec::select("gamemode", "Game mode", g(|s| s.gamemode.clone(), "creative"), vec![DdOpt::new("creative", "Creative"), DdOpt::new("survival", "Survival")]),
                FieldSpec::text("seed", "World seed", g(|s| s.seed.clone(), ""), ""),
                FieldSpec::text("motd", "Description", g(|s| s.motd.clone(), ""), ""),
                FieldSpec::text("owner", "Operator username", owner, "Breakmine username"),
                FieldSpec::select("allowLan", "Connections", if e.as_ref().is_some_and(|s| s.allow_lan) { "yes" } else { "no" }, vec![DdOpt::new("no", "This computer only"), DdOpt::new("yes", "Listen on the local network")]),
                FieldSpec::select("allowMods", "Allow client mods", if e.as_ref().is_some_and(|s| s.allow_mods) { "yes" } else { "no" }, vec![DdOpt::new("no", "No"), DdOpt::new("yes", "Yes")]),
            ],
            if existing.is_some() { "Save" } else { "Create" },
            false,
            None,
            Rc::new(move |this, v, _, cx| {
                let get = |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, s)| s.clone()).unwrap_or_default();
                let port: u16 = match get("port").trim().parse::<u32>() {
                    Ok(p) if (1024..=65535).contains(&p) => p as u16,
                    _ => return this.error("Enter a port from 1024 to 65535.", cx),
                };
                let server = hosted::HostedServer {
                    id: id.clone().unwrap_or_default(),
                    name: get("name"),
                    port,
                    world_type: get("worldType"),
                    gamemode: get("gamemode"),
                    seed: get("seed"),
                    motd: get("motd"),
                    allow_mods: get("allowMods") == "yes",
                    owner: get("owner"),
                    allow_lan: get("allowLan") == "yes",
                };
                let app = this.v.servers.app.clone();
                if app.host().status(&app).running_id.as_deref() == Some(server.id.as_str()) && !server.id.is_empty() {
                    return this.error("Stop the server before changing its settings.", cx);
                }
                match hosted::save(&app, server) {
                    Ok(saved) => {
                        let sid = format!("host-{}", saved.id);
                        if !this.store["servers"].is_array() {
                            this.store["servers"] = json!([]);
                        }
                        let list = this.store["servers"].as_array_mut().unwrap();
                        let entry = match list.iter_mut().position(|s| s["id"].as_str() == Some(&sid)) {
                            Some(i) => &mut list[i],
                            None => {
                                list.push(json!({ "id": sid, "own": true, "protocol": "binary", "lastJoined": 0 }));
                                list.last_mut().unwrap()
                            }
                        };
                        entry["name"] = json!(saved.name);
                        entry["address"] = json!(format!("127.0.0.1:{}", saved.port));
                        entry["motd"] = json!(saved.motd);
                        this.save_store();
                        this.load_hosted();
                        this.info(if id.is_some() { "Server settings saved" } else { "Local server created" }, cx);
                    }
                    Err(e) => this.error(e, cx),
                }
            }),
            window,
            cx,
        );
    }

    fn run_host_task(&mut self, cx: &mut Context<Self>, task: impl FnOnce(AppHandle, Arc<share::ShareState>) -> Result<Option<String>, String> + Send + 'static) {
        self.v.servers.busy = true;
        cx.notify();
        let (app, share) = (self.v.servers.app.clone(), self.v.servers.share.clone());
        let executor = cx.background_executor().clone();
        let timer = cx.background_executor().clone();
        // Redraw while the task runs, so install progress moves.
        cx.spawn(async move |view, cx| loop {
            timer.timer(Duration::from_millis(400)).await;
            let busy = view.update(cx, |this, cx| {
                cx.notify();
                this.v.servers.busy
            });
            if !busy.unwrap_or(false) {
                break;
            }
        })
        .detach();
        cx.spawn(async move |view, cx| {
            let result = executor.spawn(async move { task(app, share) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.servers.busy = false;
                this.v.servers.installing = false;
                this.v.servers.share_installing = false;
                match result {
                    Ok(Some(msg)) => this.info(msg, cx),
                    Ok(None) => {}
                    Err(e) => this.error(e, cx),
                }
                this.load_hosted();
                cx.notify();
            });
        })
        .detach();
    }

    fn install_runtime(&mut self, cx: &mut Context<Self>) {
        self.v.servers.installing = true;
        self.v.servers.app.clear_event("hosted-install-progress");
        self.run_host_task(cx, |app, _| {
            let _guard = app.host().installing.try_lock().map_err(|_| "Server installation is already running.".to_string())?;
            hosted::prepare(&app).map(|_| Some("Server runtime installed".into()))
        });
    }

    fn uninstall_runtime(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            "Uninstall server runtime?",
            "The downloaded server files will be removed. Your servers and worlds will stay saved.",
            "Uninstall",
            Rc::new(|this, _, cx| this.run_host_task(cx, |app, _| app.host().uninstall_runtime(&app).map(|_| Some("Server runtime uninstalled".into())))),
            window,
            cx,
        );
    }

    fn start_hosted(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.v.servers.status.as_ref().is_some_and(|s| s.runtime_ready) {
            return self.error("Install the server runtime first.", cx);
        }
        self.run_host_task(cx, move |app, _| app.host().start(&app, &id).map(|_| None));
    }

    fn stop_hosted(&mut self, cx: &mut Context<Self>) {
        self.run_host_task(cx, |app, share| {
            let _ = share.status(&app);
            share.stop();
            app.host().stop().map(|_| Some("Server stopped".into()))
        });
    }

    fn delete_hosted(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.v.servers.hosted.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
        self.confirm(
            format!("Delete {name}?"),
            "This permanently deletes the server, its world, settings and server mods. This cannot be undone.",
            "Delete server",
            Rc::new(move |this, _, cx| {
                let app = this.v.servers.app.clone();
                match app.host().delete(&app, &id) {
                    Ok(_) => {
                        let sid = format!("host-{id}");
                        if let Some(list) = this.store["servers"].as_array_mut() {
                            list.retain(|s| s["id"].as_str() != Some(&sid));
                        }
                        this.save_store();
                        this.v.servers.icons.remove(&id);
                        this.load_hosted();
                        this.info("Server deleted", cx);
                    }
                    Err(e) => this.error(e, cx),
                }
            }),
            window,
            cx,
        );
    }

    fn join_hosted(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(h) = self.v.servers.hosted.iter().find(|s| s.id == id).cloned() else { return };
        if self.current_instance().is_none() {
            return self.error("Create an instance first.", cx);
        }
        let sid = format!("host-{}", h.id);
        if !self.store["servers"].is_array() {
            self.store["servers"] = json!([]);
        }
        let list = self.store["servers"].as_array_mut().unwrap();
        if !list.iter().any(|s| s["id"].as_str() == Some(&sid)) {
            list.push(json!({ "id": sid, "name": h.name, "address": format!("127.0.0.1:{}", h.port), "motd": h.motd, "own": true, "protocol": "binary", "lastJoined": 0 }));
        }
        self.join_server(&sid, cx);
    }

    fn share_hosted(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let installed = self.v.servers.sharing.as_ref().is_some_and(|s| s.installed);
        let text = format!("Anyone with the temporary address can attempt to join while sharing is active.{}", if installed { "" } else { " The Cloudflare sharing tool will be downloaded first." });
        self.open_modal(
            "Share this server online?",
            Some(&text),
            vec![],
            "Share online",
            false,
            None,
            Rc::new(move |this, _, _, cx| {
                let id = id.clone();
                this.v.servers.share_installing = !installed;
                this.v.servers.app.clear_event("share-install-progress");
                this.run_host_task(cx, move |app, share| {
                    if !installed {
                        share.install(&app)?;
                    }
                    if app.host().status(&app).running_id.as_deref() != Some(&id) {
                        return Err("Start the local server first.".into());
                    }
                    let server = hosted::list(&app).into_iter().find(|s| s.id == id).ok_or("Server not found.")?;
                    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], server.port));
                    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map_err(|_| "The server is still starting. Try again in a moment.")?;
                    share.start(&app, &id, server.port)?;
                    Ok(None)
                });
            }),
            window,
            cx,
        );
    }

    fn pick_host_icon(&mut self, id: String, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose a server icon".into()) });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| square_icon(&b));
            let _ = view.update(cx, |this, cx| {
                match result.and_then(|png| {
                    let app = this.v.servers.app.clone();
                    app.host().set_icon(&app, &id, &png).map(|_| png)
                }) {
                    Ok(png) => {
                        this.v.servers.icons.insert(id.clone(), Some(Arc::new(gpui::Image::from_bytes(ImageFormat::Png, png))));
                        this.refresh_servers(cx);
                        this.info("Server icon saved. It will appear when the server starts.", cx);
                    }
                    Err(e) => this.error(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// `.chip.line` with a leading element.
fn chip_line(lead: AnyElement, text: impl Into<SharedString>) -> impl IntoElement {
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
        .child(lead)
        .child(text.into())
}

#[cfg(test)]
mod player_list_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn server_players_render_names_with_default_heads(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| crate::layout_test_launcher(cx));
        cx.update(|_, app| view.update(app, |launcher, _| {
            launcher.route = crate::app::Route::Servers;
            launcher.store["servers"] = json!([{ "id": "test-server", "name": "Test", "address": "example.test", "protocol": "binary" }]);
            launcher.server_checks.insert("test-server".into(), ServerCheck::Reachable(crate::server_status::Status {
                latency_ms: 5, players: Some(2), max_players: Some(20), player_names: vec!["Alex".into(), "Steve".into()], icon: None,
            }, false));
            launcher.remote_skins.insert("Alex".into(), None);
            launcher.remote_skins.insert("Steve".into(), None);
        }));
        cx.simulate_resize(gpui::size(px(1200.), px(900.)));
        cx.refresh().unwrap();
        cx.run_until_parked();
        let alex = cx.debug_bounds("server-player-Alex").expect("Alex rendered");
        let steve = cx.debug_bounds("server-player-Steve").expect("Steve rendered");
        assert!(alex.size.width >= px(24.) && steve.size.width >= px(24.));
        assert!(alex.right() <= steve.left() || alex.bottom() <= steve.top(), "player badges overlap");
    }
}
