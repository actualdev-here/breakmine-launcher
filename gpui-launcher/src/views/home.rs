//! Home (src/js/views/home.js): player, Beta art with the status strip, instance rows, servers, Mod Maker.
use super::{icon_image, scene_of};
use crate::app::{ago, plural, Route};
use crate::ui::kit::*;
use crate::ui::theme::*;
use crate::{assets, Launcher, ServerCheck};
use gpui::{AnimationExt, div, img, prelude::*, px, AnyElement, Context, FontWeight, SharedString, Window};

#[derive(Default)]
pub struct State {
    /// The new-version notice the player closed (STRIP.dismissed).
    pub dismissed: String,
}

fn hero_art(art: &str) -> std::path::PathBuf {
    assets::path(match art {
        "sky" => "assets/game/hoof_sky.png",
        "launcher" => "assets/game/launcher_art.png",
        _ => "assets/game/beta_art.webp",
    })
}

impl Launcher {
    fn instance_name(&self, id: &str) -> String {
        self.instances.iter().find(|i| i.inst.id == id).map(|i| i.inst.name.clone()).unwrap_or_else(|| "Breakmine".into())
    }

    pub fn busy(&self) -> bool {
        self.game.is_some() || self.game_starting
    }

    pub fn elapsed(&self) -> String {
        let Some(g) = &self.game else { return String::new() };
        let m = g.started.elapsed().as_secs() / 60;
        if m < 1 {
            "just started".into()
        } else if m < 60 {
            format!("{m}m")
        } else {
            format!("{}h {}m", m / 60, m % 60)
        }
    }

    pub fn play(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = id.clone();
        if let Some(s) = self.store.get_mut("settings").and_then(|s| s.as_object_mut()) {
            s.insert("selectedInstance".into(), id.into());
        }
        self.save_store();
        self.start_game(cx);
    }

    pub fn focus_game(&mut self) {
        use std::io::Write;
        if let Some(g) = &mut self.game {
            if let Some(input) = g.child.stdin.as_mut() {
                let _ = input.write_all(b"focus\n").and_then(|_| input.flush());
            }
        }
    }

    /// home.js `strip()`: launch progress, the running game, or an offline download.
    fn strip(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shell = |d: gpui::Div| {
            d.absolute()
                .left(px(12.))
                .right(px(12.))
                .bottom(px(12.))
                .min_h(px(52.))
                .flex()
                .items_center()
                .gap(px(12.))
                .pl(px(14.))
                .pr(px(10.))
                .py(px(8.))
                .border_1()
                .border_color(ca(0xffffff1a))
                .rounded(px(12.))
                .bg(ca(0x080808eb))
        };
        if self.game_starting {
            let name = self.instance_name(&self.selected);
            let segment = div().absolute().top_0().bottom_0().w(gpui::relative(0.3)).rounded_full().bg(c(ACCENT));
            let segment = if crate::ui::motion::reduced() { segment.left_0().into_any_element() } else {
                segment.with_animation("strip-bar", gpui::Animation::new(std::time::Duration::from_millis(1400)).repeat(), |el, t| el.left(gpui::relative(-0.3 + 1.3 * t))).into_any_element()
            };
            return Some(
                shell(div())
                    .child(div().min_w(px(46.)).font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(17.)).text_color(c(0xffffff)).child(spinner("strip-spin", c(0xffffff))))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).truncate().child(div().flex().child(div().font_weight(FontWeight::BOLD).child(format!("Starting {name}"))).child(div().text_color(c(DIM)).child(" · Preparing the game"))))
                            .child(
                                div().relative().h(px(6.)).rounded_full().bg(ca(0xffffff1f)).overflow_hidden().child(
                                    segment,
                                ),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(g) = &self.game {
            let name = self.instance_name(&g.instance_id);
            let status = g.status.lock().ok().and_then(|v| v.clone()).filter(|(_, at)| at.elapsed().as_secs() < 12).map(|(v, _)| {
                let state = if v.paused { "Paused".to_string() } else if v.singleplayer == Some(true) { "Singleplayer".to_string() } else if v.connected { "Multiplayer".to_string() } else { v.state.unwrap_or_else(|| "In game".into()) };
                if v.connected && !v.players.is_empty() { format!("{state} · {}", v.players.join(", ")) } else { state }
            });
            return Some(
                shell(div())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .flex_1()
                            .min_w_0()
                            .child(div().flex().text_size(px(13.)).child("Playing ").child(div().font_weight(FontWeight::BOLD).child(name)))
                            .child(div().font_family(MONO).text_size(px(11.)).text_color(c(MUTED)).truncate().child(match status { Some(status) => format!("{} · {status}", self.elapsed()), None => self.elapsed() })),
                    )
                    .child(Btn::new("strip-show", "Show").sm().icon("external").icon_size(12.).on_click(cx.listener(|this, _, _, _| this.focus_game())))
                    .child(Btn::new("strip-stop", "Stop").stop().sm().icon("stop").icon_size(12.).on_click(cx.listener(|this, _, _, cx| {
                        this.stop_game();
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        if let Some(task) = &self.offline_task {
            let p = task.progress.lock().map(|p| (p.received, p.total, p.phase)).ok();
            let label = match p {
                Some((r, Some(t), _)) if t > 0 => format!("{}%", r * 100 / t),
                Some((r, _, phase)) if r > 0 => format!("{phase} · {:.1} MB", r as f64 / 1_048_576.),
                Some((_, _, phase)) => phase.to_string(),
                None => String::new(),
            };
            return Some(
                shell(div())
                    .child(spinner("dl-spin", c(FG)))
                    .child(div().flex_1().flex().text_size(px(13.)).child("Downloading Breakmine · ").child(div().text_color(c(DIM)).child(label)))
                    .into_any_element(),
            );
        }
        // A newer offline build is out (and an instance follows the newest).
        let installed = crate::offline_download::current(&crate::launcher_data()).map(|(v, _)| v);
        let latest = self.latest_offline();
        let uses_newest = self.instances.iter().any(|i| self.build_of(&i.inst) == "offline" && i.inst.game_version.is_empty());
        if let (Some(i), Some(l)) = (installed, latest) {
            if uses_newest && i != l && self.v.home.dismissed != l {
                let dismiss = l.clone();
                return Some(
                    shell(div())
                        .child(icon("gift", 14., c(ACCENT)))
                        .child(div().flex_1().min_w_0().flex().text_size(px(13.)).child(div().font_weight(FontWeight::BOLD).child(format!("Breakmine {l}"))).child(" is out ").child(div().text_color(c(DIM)).child(format!("· you have {i}"))))
                        .child(Btn::new("update-game", "Download").primary().sm().on_click(cx.listener(|this, _, _, cx| {
                            this.start_offline_download(cx);
                            this.v.settings_reset_latest();
                        })))
                        .child(icon_btn("dismiss-update", "close", true, false).on_click(cx.listener(move |this, _, _, cx| {
                            this.v.home.dismissed = dismiss.clone();
                            cx.notify();
                        })))
                        .into_any_element(),
                );
            }
        }
        None
    }

    fn inst_row(&mut self, n: usize, item: &crate::instances::InstanceView, show_version: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let inst = &item.inst;
        let id = inst.id.clone();
        let running = self.game.as_ref().is_some_and(|g| g.instance_id == id);
        let launching = self.game_starting && self.selected == id;
        let busy = self.busy();
        let (opts, value) = self.version_options(inst);
        let dd_id: SharedString = format!("ver-{id}").into();
        let open = self.dd_open.as_ref() == Some(&dd_id);
        let (tid, vid) = (dd_id.clone(), id.clone());
        let dropdown = Dropdown::new(dd_id, opts, value, open)
            .width(170.)
            .height(36.)
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_dd(&tid, cx)))
            .on_pick(cx.listener(move |this, v: &SharedString, _, cx| this.set_instance_version(&vid, v, cx)));

        let mut buttons = div().flex().items_center().gap(px(12.));
        if launching {
            buttons = buttons.child(Btn::new(SharedString::from(format!("cancel-{id}")), "Starting").dark().min_w(92.).leading(spinner(SharedString::from(format!("rs-{id}")), c(FG))));
        } else if running {
            buttons = buttons
                .child(Btn::new(SharedString::from(format!("show-{id}")), "Show").icon("external").icon_size(12.).min_w(92.).on_click(cx.listener(|this, _, _, _| this.focus_game())))
                .child(Btn::new(SharedString::from(format!("stop-{id}")), "Stop").stop().icon("stop").icon_size(12.).min_w(92.).on_click(cx.listener(|this, _, _, cx| {
                    this.stop_game();
                    cx.notify();
                })));
        } else {
            let pid = id.clone();
            buttons = buttons.child(
                Btn::new(SharedString::from(format!("play-{id}")), "Play")
                    .primary()
                    .icon("play")
                    .icon_size(12.)
                    .min_w(92.)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.play(pid.clone(), cx))),
            );
        }
        let sid = id.clone();
        let meta = format!(
            "{}{}",
            if item.mods_count > 0 { format!("{} · ", plural(item.mods_count, "mod")) } else { String::new() },
            if inst.last_played > 0 { ago(inst.last_played) } else { "Never played".into() }
        );
        let mut icon_box = div().relative().size(px(40.)).flex_none();
        if let Some(i) = icon_image(inst) {
            icon_box = icon_box.child(img(i).size(px(40.)));
        }
        if running {
            icon_box = icon_box.child(div().absolute().right(px(-2.)).bottom(px(-2.)).child(sq(SharedString::from(format!("run-{id}")), false, true)));
        }
        div()
            .id(SharedString::from(format!("row-{id}")))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(10.))
            .border_1()
            .border_color(c(LINE))
            .rounded(px(14.))
            .bg(c(CARD))
            .hover(|s| s.border_color(c(0x2a2a2a)))
            .child(icon_box)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .min_w_0()
                    .flex_1()
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).truncate().child(inst.name.clone()))
                    .child(div().font_family(MONO).text_size(px(12.)).text_color(c(DIM)).truncate().child(meta)),
            )
            .when(show_version, |d| d.child(dropdown))
            .child(buttons)
            .child(sq_btn(SharedString::from(format!("gear-{id}")), "gear", SqKind::Default).on_click(cx.listener(move |this, _, _, cx| {
                this.v.instances.next = Some(sid.clone());
                this.go(Route::Instances, cx);
            })))
            .with_animation(
                SharedString::from(format!("row-in-{n}")),
                gpui::Animation::new(std::time::Duration::from_millis(if crate::ui::motion::reduced() { 1 } else { 380 + 80 + 40 * n.min(12) as u64 })),
                move |el, t| {
                    // Each row pops in 40ms after the one above (card-in, with a delay).
                    let total = 460. + 40. * n.min(12) as f32;
                    let delay = (80. + 40. * n.min(12) as f32) / total;
                    let k = gpui::ease_out_quint()(((t - delay) / (1. - delay)).clamp(0., 1.));
                    el.opacity(k).mt(px(12. * (1. - k)))
                },
            )
    }

    fn server_cover(&self, id: &str) -> AnyElement {
        if let Some(ServerCheck::Reachable(status, _)) = self.server_checks.get(id) {
            if let Some((_, bytes)) = &status.icon {
                return assets::rounded_cover_bytes(bytes.clone(), 8., 0.5, 0.68).into_any_element();
            }
        }
        assets::rounded_cover(scene_of(id), 8., 0.5, 0.68).into_any_element()
    }

    pub fn player_text(&self, id: &str) -> Option<String> {
        match self.server_checks.get(id) {
            Some(ServerCheck::Reachable(s, _)) if s.players.is_some() || s.max_players.is_some() => Some(format!(
                "{}/{} players",
                s.players.map(|n| n.to_string()).unwrap_or("?".into()),
                s.max_players.map(|n| n.to_string()).unwrap_or("?".into())
            )),
            _ => None,
        }
    }

    pub fn home_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vh = f32::from(window.viewport_size().height);
        let vw = f32::from(window.viewport_size().width);
        // @media (max-width: 1100px) / (max-width: 980px)
        let (left_w, side_w, gap) = if vw <= 650. { (100., 240., 10.) } else if vw <= 1100. { (150., 240., 12.) } else { (180., 270., 18.) };
        let stacked = vw <= 980.;
        let stage_h = (vh * 0.38).clamp(220., 380.);
        let acct_name = self.active_account().map(|a| a.1).unwrap_or_else(|| "Player".into());
        let art = self.store.pointer("/settings/homeArt").and_then(|v| v.as_str()).unwrap_or("classic").to_string();

        // Left: the player.
        let player = div()
            .w(px(left_w))
            .flex_none()
            .h(px(stage_h))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .pt(px(12.))
            .pb(px(14.))
            .child(div().relative().w_full().flex_1().min_h_0().child(self.home_skin(cx)))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).max_w_full().truncate().child(acct_name))
            .child(link("home-skin", "Change skin").text_size(px(12.)).on_click(cx.listener(|this, _, _, cx| this.go(Route::Skins, cx))));

        // Middle: art, instances.
        let mut list: Vec<crate::instances::InstanceView> = self.instances.clone();
        list.sort_by(|a, b| b.inst.last_played.cmp(&a.inst.last_played).then(b.inst.pinned.cmp(&a.inst.pinned)));
        let mut rows = div().flex().flex_col().gap(px(8.));
        if list.is_empty() {
            rows = rows.child(empty_box("No instances yet."));
        }
        for (n, item) in list.iter().enumerate() {
            rows = rows.child(self.inst_row(n, item, !stacked, cx));
        }
        let strip = self.strip(cx);
        let main = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div().relative().h(px(stage_h)).child(
                    div()
                        .debug_selector(|| "home-art".into())
                        .absolute()
                        .inset_0()
                        .rounded(px(18.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(c(0x1f1f1f))
                        .child(assets::rounded_cover(hero_art(&art), 17., 0.5, 0.45))
                        .children(strip),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(10.))
                    .child(sec_title("Instances"))
                    .child(div().ml_auto().child(link("home-manage", "Manage").child(icon("arrow-right", 12., c(0xd4d4d4))).on_click(cx.listener(|this, _, _, cx| this.go(Route::Instances, cx))))),
            )
            .child(rows);

        // Right: servers and the Mod Maker.
        let servers: Vec<(String, String, String)> = self.store["servers"]
            .as_array()
            .into_iter()
            .flatten()
            .take(4)
            .filter_map(|s| Some((s["id"].as_str()?.to_string(), s["name"].as_str().unwrap_or("").to_string(), s["address"].as_str().unwrap_or("").to_string())))
            .collect();
        let can_join = !self.busy() && !self.instances.is_empty();
        let mut srv = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(14.))
            .border_1()
            .border_color(c(LINE))
            .rounded(px(16.))
            .bg(c(CARD))
            .child(pixel_text("Servers", 16.).mb(px(2.)));
        if servers.is_empty() {
            srv = srv.child(div().text_size(px(13.)).text_color(c(DIM)).child("No servers yet."));
        }
        for (id, name, address) in servers {
            let players = self.player_text(&id).map(|p| format!(" · {p}")).unwrap_or_default();
            let join = id.clone();
            srv = srv.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(8.))
                    .rounded(px(12.))
                    .bg(c(0x0d0d0d))
                    .child(div().size(px(36.)).flex_none().rounded(px(8.)).overflow_hidden().child(self.server_cover(&id)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .min_w_0()
                            .flex_1()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).truncate().child(name))
                            .child(div().font_family(MONO).text_size(px(12.)).text_color(c(DIM)).truncate().child(format!("{address}{players}"))),
                    )
                    .child(Btn::new(SharedString::from(format!("join-{id}")), "Join").primary().sm().disabled(!can_join).on_click(cx.listener(move |this, _, _, cx| this.join_server(&join, cx)))),
            );
        }
        srv = srv.child(Btn::new("home-servers", "Manage servers").sm().full().on_click(cx.listener(|this, _, _, cx| this.go(Route::Servers, cx))));
        let make = div()
            .id("home-make")
            .flex()
            .items_center()
            .gap(px(12.))
            .p(px(14.))
            .border_1()
            .border_color(c(LINE))
            .rounded(px(16.))
            .bg(c(CARD))
            .cursor_pointer()
            .hover(|s| s.border_color(c(0x2a2a2a)))
            .on_click(cx.listener(|this, _, _, cx| this.go(Route::Create, cx)))
            .child(patchwork(22.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .min_w_0()
                    .flex_1()
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Mod Maker"))
                    .child(div().text_size(px(12.)).text_color(c(DIM)).child("Paint blocks, add recipes, test in one click")),
            )
            .child(icon("arrow-right", 12., c(FG)));
        let side = div().flex().flex_col().gap(px(12.)).child(srv).child(make);
        let side = if stacked { side.w_full() } else { side.w(px(side_w)).flex_none() };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(18.))
            .px(px(if vw <= 650. { 14. } else { 28. }))
            .pt(px(20.))
            .pb(px(22.))
            .child(if stacked {
                div().flex().flex_col().gap(px(gap)).child(div().flex().items_start().gap(px(gap)).child(player).child(main)).child(side)
            } else {
                div().flex().items_start().gap(px(gap)).child(player).child(main).child(side)
            })
    }

    /// home.js `join`: launch the selected instance and connect.
    pub fn join_server(&mut self, server_id: &str, cx: &mut Context<Self>) {
        if let Some(list) = self.store["servers"].as_array_mut() {
            for s in list.iter_mut() {
                if s["id"].as_str() == Some(server_id) {
                    s["lastJoined"] = crate::util::now().into();
                }
            }
        }
        self.join_target = Some(server_id.to_string());
        self.save_store();
        self.start_game(cx);
    }
}

