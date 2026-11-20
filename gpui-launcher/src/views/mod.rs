//! One module per page of the web UI (src/js/views/*.js).
pub mod create;
pub mod home;
pub mod instances;
pub mod mods;
pub mod servers;
pub mod settings;
pub mod signin;
pub mod skins;

use crate::app::Route;
use crate::ui::kit::DdOpt;
use crate::Launcher;
use gpui::{AnyElement, Context, IntoElement, SharedString, Window};

/// UI-only state of every page (what each JS view keeps in its local `L`).
pub struct ViewState {
    pub home: home::State,
    pub instances: instances::State,
    pub mods: mods::State,
    pub servers: servers::State,
    pub skins: skins::State,
    pub create: create::State,
    pub settings: settings::State,
    pub signin: signin::State,
    /// Archived game versions (None until loaded).
    pub versions: Option<Vec<crate::versions::GameVersion>>,
    pub versions_loading: bool,
}

impl ViewState {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        Self {
            home: home::State::default(),
            instances: instances::State::new(cx),
            mods: mods::State::new(cx),
            servers: servers::State::new(cx),
            skins: skins::State::new(cx),
            create: create::State::new(cx),
            settings: settings::State::default(),
            signin: signin::State::new(cx),
            versions: None,
            versions_loading: false,
        }
    }
}

impl ViewState {
    /// After an offline download, the "latest" check runs again.
    pub fn settings_reset_latest(&mut self) {
        self.settings.reset_latest();
    }
}

impl Launcher {
    pub fn render_route(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.route {
            Route::Home => self.home_page(window, cx).into_any_element(),
            Route::Instances => self.instances_page(window, cx).into_any_element(),
            Route::Mods => self.mods_page(window, cx).into_any_element(),
            Route::Servers => self.servers_page(window, cx).into_any_element(),
            Route::Skins => self.skins_page(window, cx).into_any_element(),
            Route::Create => self.create_page(window, cx).into_any_element(),
            Route::Settings => self.settings_page(window, cx).into_any_element(),
            Route::Signin => self.signin_page(window, cx).into_any_element(),
        }
    }

    /// The `mount()` of each JS view: load what the page needs.
    pub fn on_route(&mut self, route: Route, cx: &mut Context<Self>) {
        match route {
            Route::Home => {
                self.refresh_instances_in_background(cx);
                self.check_offline_quiet(cx);
                if !self.server_checked {
                    self.refresh_servers(cx);
                }
                self.load_versions(cx);
            }
            Route::Instances => self.instances_mount(cx),
            Route::Mods => self.mods_mount(cx),
            Route::Servers => self.servers_mount(cx),
            Route::Skins => self.skins_mount(cx),
            Route::Create => self.create_mount(cx),
            Route::Settings => self.settings_mount(cx),
            Route::Signin => self.refresh_accounts(),
        }
    }

    pub fn on_escape(&mut self, cx: &mut Context<Self>) {
        if self.route == Route::Instances && self.v.instances.open {
            self.v.instances.open = false;
            cx.notify();
        }
    }

    /// The archive listing for version pickers: cached first, then refreshed once per session.
    pub fn load_versions(&mut self, cx: &mut Context<Self>) {
        if self.v.versions.is_some() || self.v.versions_loading {
            return;
        }
        let data = crate::launcher_data();
        let cached = crate::versions::cached(&data);
        if !cached.is_empty() {
            self.v.versions = Some(cached);
        }
        self.v.versions_loading = true;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let fresh = executor.spawn(async move { crate::versions::refresh(&data) }).await;
            let _ = view.update(cx, |this, cx| {
                this.v.versions_loading = false;
                match fresh {
                    Ok(list) => this.v.versions = Some(list),
                    Err(_) => {
                        if this.v.versions.is_none() {
                            this.v.versions = Some(Vec::new());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Options for an instance's version picker (home.js `versionDropdown`).
    pub fn version_options(&self, inst: &crate::instances::Instance) -> (Vec<DdOpt>, SharedString) {
        let b = self.build_of(inst);
        let value: SharedString = if b == "offline" && !inst.game_version.is_empty() { format!("v:{}", inst.game_version).into() } else { b.clone().into() };
        let latest = crate::offline_download::current(&crate::launcher_data()).map(|(v, _)| v);
        let newest_label = latest.clone().unwrap_or_else(|| "Newest".into());
        let mut opts = vec![DdOpt::new("online", "Latest (online)"), DdOpt::new("offline", format!("{newest_label} (offline)"))];
        let data = crate::launcher_data();
        if let Some(list) = &self.v.versions {
            for v in list {
                if Some(&v.version) == latest.as_ref() {
                    continue;
                }
                let mark = if crate::versions::installed(&data, &v.version) { " ✓" } else { "" };
                opts.push(DdOpt::new(format!("v:{}", v.version), format!("{}{mark}", v.version)));
            }
        }
        if b == "local" || b == "custom" {
            opts.insert(0, DdOpt::new(b.clone(), if b == "local" { "Local dev server" } else { "Custom URL" }));
        }
        if !opts.iter().any(|o| o.value == value) {
            opts.push(DdOpt::new(value.clone(), if inst.game_version.is_empty() { value.to_string() } else { inst.game_version.clone() }));
        }
        if self.v.versions.is_none() {
            opts.push(DdOpt::new("__loading", "Loading versions…"));
        }
        (opts, value)
    }

    /// Pick a version for an instance: "online", "offline" (newest) or "v:<version>" (pinned).
    pub fn set_instance_version(&mut self, id: &str, value: &str, cx: &mut Context<Self>) {
        if value == "__loading" {
            return;
        }
        let game_dir = self.game_dir.clone();
        let result = match value.strip_prefix("v:") {
            Some(version) => crate::instances::read(&game_dir, id).and_then(|mut inst| {
                inst.build = "offline".into();
                inst.game_version = version.to_string();
                crate::instances::write(&game_dir, &inst).map(|_| inst)
            }),
            None => crate::instances::set_build(&game_dir, id, value),
        };
        match result {
            Ok(_) => self.refresh(),
            Err(e) => self.error(e, cx),
        }
        self.dd_open = None;
        cx.notify();
    }

    /// "default" follows Settings.build.
    pub fn build_of(&self, inst: &crate::instances::Instance) -> String {
        if inst.build.is_empty() || inst.build == "default" {
            self.store.pointer("/settings/build").and_then(|v| v.as_str()).unwrap_or("online").to_string()
        } else {
            inst.build.clone()
        }
    }

    /// `buildLabel()` from state.js.
    pub fn build_label(&self, inst: &crate::instances::Instance) -> String {
        let b = self.build_of(inst);
        if b == "offline" && !inst.game_version.is_empty() {
            return format!("Breakmine {}", inst.game_version);
        }
        match b.as_str() {
            "offline" => "Offline",
            "local" => "Local dev server",
            "custom" => "Custom URL",
            _ => "Online",
        }
        .into()
    }

    pub fn toggle_dd(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        if self.dd_open.as_ref() == Some(id) {
            if self.dd_closing.is_some() { return; }
            if crate::ui::motion::reduced() { self.dd_open = None; cx.notify(); return; }
            let started = std::time::Instant::now();
            self.dd_closing = Some((id.clone(), started));
            crate::ui::motion::dropdown_close_start(id.clone(), started);
            let id = id.clone();
            let timer = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
                timer.timer(std::time::Duration::from_millis(120)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.dd_closing.as_ref() == Some(&(id.clone(), started)) {
                        crate::ui::motion::dropdown_close_end(&id);
                        this.dd_open = None; this.dd_closing = None; cx.notify();
                    }
                });
            }).detach();
        } else {
            if let Some((old, _)) = self.dd_closing.take() { crate::ui::motion::dropdown_close_end(&old); }
            crate::ui::motion::reset(&format!("ddi:{id}:"));
            crate::ui::motion::reset(&format!("dd-in:{id}"));
            self.dd_open = Some(id.clone());
        }
        cx.notify();
    }
}

/// The pixel icon an instance uses (state.js `iconOf`).
pub fn icon_of(inst: &crate::instances::Instance) -> &'static str {
    const ICONS: [&str; 30] = [
        "grass", "diamond", "emerald", "sapphire", "gold", "crown", "heart", "star", "sun", "moon", "house", "tower", "tree", "sprout", "turnip", "sword", "shield", "hammer", "anvil", "key", "book", "scroll", "potion", "pearl", "lens", "hourglass", "trophy", "skull", "coal", "chef",
    ];
    if let Some(i) = ICONS.iter().find(|n| **n == inst.icon) {
        return i;
    }
    ICONS[(hash(&inst.id) % ICONS.len() as u32) as usize]
}

pub const INSTANCE_ICONS: [&str; 30] = [
    "grass", "diamond", "emerald", "sapphire", "gold", "crown", "heart", "star", "sun", "moon", "house", "tower", "tree", "sprout", "turnip", "sword", "shield", "hammer", "anvil", "key", "book", "scroll", "potion", "pearl", "lens", "hourglass", "trophy", "skull", "coal", "chef",
];

fn hash(id: &str) -> u32 {
    let mut h: u32 = 7;
    for ch in id.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(ch as u32);
    }
    h
}

/// A Breakmine scene picked from an id (state.js `coverOf` without a custom cover).
pub fn scene_of(id: &str) -> std::path::PathBuf {
    const SCENES: [&str; 8] = [
        "assets/game/pano/day_1.webp", "assets/game/pano/sunset_1.webp", "assets/game/snow.webp", "assets/game/pano/day_0.webp", "assets/game/pano/sunset_0.webp", "assets/game/pano/nether_0.webp", "assets/game/pano/day_3.webp", "assets/game/pano/sunset_3.webp",
    ];
    crate::assets::path(SCENES[(hash(id) % 8) as usize])
}

/// The instance icon as a sharp pixel image.
pub fn icon_image(inst: &crate::instances::Instance) -> Option<std::sync::Arc<gpui::Image>> {
    crate::assets::pixel(&format!("assets/icons/{}.png", icon_of(inst)), 8)
}
