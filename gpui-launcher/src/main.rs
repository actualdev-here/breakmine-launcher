mod account_api;
mod app;
mod assets;
mod catalog;
mod creator;
mod credential_store;
mod game_launch;
mod frame_clock;
mod host;
#[allow(dead_code)]
mod instances;
mod local_accounts;
mod local_servers;
mod memory_cache;
mod mod_share;
mod oauth;
mod pano;
mod offline_download;
mod server_status;
mod skin3d;
mod skin_preview;
mod store;
#[path = "../game-host/src/telemetry.rs"]
mod telemetry;
mod text_input;
mod ui;
#[allow(dead_code)]
mod util;
mod versions;
mod views;

use app::Route;

use gpui::{
    prelude::*, px, size, App, Application, Bounds, Context, Window, WindowBounds,
    WindowOptions,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process::Child,
    sync::{
        atomic::AtomicBool,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
enum ServerCheck {
    Checking,
    Reachable(server_status::Status, #[allow(dead_code)] bool),
    Failed,
}

struct GameSession {
    child: Child,
    instance_id: String,
    started: Instant,
    status: Arc<Mutex<Option<(telemetry::Snapshot, Instant)>>>,
}

struct OfflineTask {
    #[allow(dead_code)]
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<offline_download::Progress>>,
}

pub(crate) struct Launcher {
    route: Route,
    top_open: Option<&'static str>,
    dd_open: Option<gpui::SharedString>,
    dd_closing: Option<(gpui::SharedString, Instant)>,
    modal: Option<app::Modal>,
    modal_closing: Option<Instant>,
    toasts: Vec<app::Toast>,
    toast_seq: u64,
    remote_skins: HashMap<String, Option<Vec<u8>>>,
    join_target: Option<String>,
    /// Minimize the launcher on the next frame (Settings → While you play).
    minimize_pending: bool,
    v: views::ViewState,
    store: Value,
    game_dir: String,
    selected: String,
    notice: Option<String>,
    account_prefs: local_accounts::Accounts,
    game: Option<GameSession>,
    game_starting: bool,
    offline_task: Option<OfflineTask>,
    server_checks: HashMap<String, ServerCheck>,
    server_generation: u64,
    server_checked: bool,
    skin_previews: HashMap<String, Arc<gpui::Image>>,
    skins_checked: bool,
    skins_loading: bool,
    instances: Vec<instances::InstanceView>,
    instance_refresh_generation: u64,
}

pub(crate) fn read_json(path: PathBuf) -> Value {
    fs::read(path)
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or(Value::Null)
}
pub(crate) fn gpui_accounts_path() -> PathBuf {
    launcher_data().join("gpui-accounts.json")
}
pub(crate) fn gpui_servers_path() -> PathBuf {
    launcher_data().join("gpui-servers.json")
}
/// Launcher settings and state the GPUI UI changes (Tauri's launcher.json stays untouched).
pub(crate) fn gpui_store_path() -> PathBuf {
    launcher_data().join("gpui-launcher.json")
}
pub(crate) fn launcher_data() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join("com.breakmine.launcher")
}
impl Launcher {
    fn load(cx: &mut Context<Self>) -> Self {
        // GPUI keeps its own copy of the launcher store once it changes anything.
        let own = read_json(gpui_store_path());
        let has_own = own.is_object();
        let mut store = if has_own { own } else { read_json(launcher_data().join("launcher.json")) };
        let legacy_error = match credential_store::protect_legacy_accounts(&mut store) {
            Ok(true) if has_own => crate::store::write_json(&gpui_store_path(), &credential_store::without_legacy_tokens(&store)).err(),
            Ok(_) => None,
            Err(error) => Some(error),
        };
        let game_dir = store
            .pointer("/settings/gameDir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".breakmine")
            });
        let game_dir = game_dir.to_string_lossy().into_owned();
        let instances = instances::list(&game_dir);
        let selected = store
            .pointer("/settings/selectedInstance")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let (account_prefs, account_error) = match local_accounts::load(&gpui_accounts_path()) {
            Ok(accounts) => (accounts, None),
            Err(error) => (local_accounts::load_metadata(&gpui_accounts_path()).unwrap_or_default(), Some(error)),
        };
        let mut launcher = Self {
            route: Route::Home,
            top_open: None,
            dd_open: None,
            dd_closing: None,
            modal: None,
            modal_closing: None,
            toasts: Vec::new(),
            toast_seq: 0,
            remote_skins: HashMap::new(),
            join_target: None,
            minimize_pending: false,
            v: views::ViewState::new(cx),
            store,
            game_dir,
            selected,
            notice: account_error.or(legacy_error),
            account_prefs,
            game: None,
            game_starting: false,
            offline_task: None,
            server_checks: HashMap::new(),
            server_generation: 0,
            server_checked: false,
            skin_previews: HashMap::new(),
            skins_checked: false,
            skins_loading: false,
            instances,
            instance_refresh_generation: 0,
        };
        launcher.refresh_skins(cx);
        launcher.refresh_servers(cx);
        // Debug aid: open on a page (home, instances, mods, servers, skins, create, settings, signin).
        if let Ok(r) = std::env::var("BM_ROUTE") {
            let route = match r.as_str() {
                "instances" => Route::Instances,
                "mods" => Route::Mods,
                "servers" => Route::Servers,
                "skins" => Route::Skins,
                "create" => Route::Create,
                "settings" => Route::Settings,
                "signin" => Route::Signin,
                _ => Route::Home,
            };
            launcher.route = route;
            launcher.on_route(route, cx);
        } else {
            launcher.on_route(Route::Home, cx);
        }
        launcher
    }
    fn start_offline_download(&mut self, cx: &mut Context<Self>) {
        if self.offline_task.is_some() {
            return;
        }
        let source = self
            .store
            .pointer("/settings/offlineSource")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("https://datastore.breakmine.com/")
            .to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(offline_download::Progress {
            phase: "Checking version",
            ..Default::default()
        }));
        self.offline_task = Some(OfflineTask {
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        self.notice = None;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let result = executor
                .spawn(async move {
                    offline_download::download(&source, &launcher_data(), &cancel, &progress)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.offline_task = None;
                this.notice = Some(match result {
                    Ok(version) => format!("Offline build {version} downloaded."),
                    Err(error) => error,
                });
                cx.notify();
            });
        })
        .detach();
        let timer = cx.background_executor().clone();
        cx.spawn(async move |view, cx| loop {
            timer.timer(Duration::from_millis(250)).await;
            if !view
                .update(cx, |this, cx| {
                    cx.notify();
                    this.offline_task.is_some()
                })
                .unwrap_or(false)
            {
                break;
            }
        })
        .detach();
        cx.notify();
    }
    fn start_game(&mut self, cx: &mut Context<Self>) {
        if self.game_starting || self.game.is_some() {
            return;
        }
        let Some(instance) = self
            .instances
            .iter()
            .find(|item| item.inst.id == self.selected)
            .or_else(|| self.instances.first())
        else {
            self.notice = Some("Create an instance first.".into());
            cx.notify();
            return;
        };
        let id = instance.inst.id.clone();
        let selected_account = self.selected_account_id();
        let game_dir = self.game_dir.clone();
        let data_dir = launcher_data();
        let mut store = self.store.clone();
        if let Some(join) = self.join_target.take() {
            store["__join"] = Value::String(join);
        }
        match local_servers::load(&gpui_servers_path()) {
            Ok(local) => {
                if !store["servers"].is_array() {
                    store["servers"] = serde_json::json!([]);
                }
                if let Some(servers) = store["servers"].as_array_mut() {
                    for server in local {
                        if let Ok(value) = serde_json::to_value(server) {
                            servers.push(value);
                        }
                    }
                }
            }
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
                return;
            }
        }
        let prefs = match local_accounts::load(&gpui_accounts_path()) {
            Ok(prefs) => prefs,
            Err(error) => {
                let metadata = local_accounts::load_metadata(&gpui_accounts_path()).unwrap_or_default();
                let local_offline = metadata.offline_profiles.iter().any(|account| account.id == selected_account);
                let legacy_offline = store["accounts"].as_array().into_iter().flatten().any(|account| account["id"].as_str() == Some(&selected_account) && account["offline"].as_bool() == Some(true));
                if local_offline || legacy_offline { metadata } else {
                    self.notice = Some(error);
                    cx.notify();
                    return;
                }
            }
        };
        self.game_starting = true;
        let executor = cx.background_executor().clone();
        cx.notify();
        cx.spawn(async move |view, cx| {
            let game_id = id.clone();
            let result = executor
                .spawn(async move {
                    // Download a pinned older version on first Play.
                    if let Ok(inst) = instances::read(&game_dir, &game_id) {
                        let build = if inst.build.is_empty() || inst.build == "default" { store.pointer("/settings/build").and_then(Value::as_str).unwrap_or("online").to_string() } else { inst.build.clone() };
                        if build == "offline" && !inst.game_version.is_empty() && !versions::installed(&data_dir, &inst.game_version) {
                            versions::install(&data_dir, &inst.game_version, |_| {})?;
                        }
                    }
                    let config = game_launch::prepare(
                        &game_dir,
                        &data_dir,
                        &game_id,
                        &selected_account,
                        &store,
                        &prefs,
                    )?;
                    game_launch::start(config)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.game_starting = false;
                match result {
                    Ok(started) => {
                        this.game = Some(GameSession {
                            child: started.child,
                            status: started.status,
                            instance_id: id,
                            started: Instant::now(),
                        });
                        this.watch_game(cx);
                        let on_launch = this.store.pointer("/settings/onLaunch").and_then(Value::as_str).unwrap_or("minimize");
                        this.minimize_pending = on_launch == "minimize" || on_launch == "close";
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn watch_game(&self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().clone();
        cx.spawn(async move |view, cx| loop {
            timer.timer(Duration::from_secs(1)).await;
            let running = view
                .update(cx, |this, cx| {
                    let Some(game) = this.game.as_mut() else {
                        return false;
                    };
                    match game.child.try_wait() {
                        Ok(Some(status)) => {
                            let game = this.game.take().unwrap();
                            if let Ok(mut instance) =
                                instances::read(&this.game_dir, &game.instance_id)
                            {
                                instance.playtime = instance
                                    .playtime
                                    .saturating_add(game.started.elapsed().as_secs());
                                instance.last_played = util::now();
                                let _ = instances::write(&this.game_dir, &instance);
                            }
                            this.refresh();
                            if !status.success() {
                                this.notice = Some("The game closed unexpectedly.".into());
                            }
                            // Session summary for the inbox (Settings → Session summaries).
                            let secs = game.started.elapsed().as_secs();
                            if this.store.pointer("/settings/notifications").and_then(Value::as_bool).unwrap_or(true) && secs >= 60 {
                                let name = this.instances.iter().find(|i| i.inst.id == game.instance_id).map(|i| i.inst.name.clone()).unwrap_or_else(|| "Breakmine".into());
                                let (h, m) = (secs / 3600, (secs % 3600) / 60);
                                let took = if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") };
                                if !this.store["notes"].is_array() {
                                    this.store["notes"] = serde_json::json!([]);
                                }
                                if let Some(notes) = this.store["notes"].as_array_mut() {
                                    notes.insert(0, serde_json::json!({ "id": util::new_id("n"), "text": format!("Played {name} for {took}"), "at": util::now(), "read": false }));
                                    notes.truncate(30);
                                }
                                this.save_store();
                            }
                            cx.notify();
                            false
                        }
                        Ok(None) => { cx.notify(); true },
                        Err(error) => {
                            this.notice = Some(format!("Couldn't check game status: {error}"));
                            cx.notify();
                            false
                        }
                    }
                })
                .unwrap_or(false);
            if !running {
                break;
            }
        })
        .detach();
    }
    fn stop_game(&mut self) {
        use std::io::Write;
        if let Some(game) = &mut self.game {
            match game
                .child
                .stdin
                .as_mut()
                .ok_or_else(|| "Game connection is closed.".to_string())
                .and_then(|input| {
                    input
                        .write_all(b"stop\n")
                        .and_then(|_| input.flush())
                        .map_err(|e| e.to_string())
                }) {
                Ok(()) => self.notice = Some("Closing game...".into()),
                Err(error) => self.notice = Some(format!("Couldn't close game: {error}")),
            }
        }
    }
    fn accounts(&self) -> Vec<(String, String, bool, bool, bool)> {
        let mut result: Vec<_> = self.store["accounts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|account| {
                let id = account["id"].as_str()?.to_string();
                let name = account["name"].as_str()?.to_string();
                let offline = account["offline"].as_bool().unwrap_or(false);
                let expired = !offline
                    && account["expires"]
                        .as_u64()
                        .is_some_and(|expiry| expiry <= util::now());
                Some((id, name, offline, expired, false))
            })
            .collect();
        result.extend(
            self.account_prefs
                .offline_profiles
                .iter()
                .map(|account| (account.id.clone(), account.name.clone(), true, false, true)),
        );
        result.extend(self.account_prefs.online_profiles.iter().map(|account| {
            (
                account.id.clone(),
                account.name.clone(),
                false,
                account.expires <= util::now(),
                true,
            )
        }));
        result
    }
    fn selected_account_id(&self) -> String {
        let accounts = self.accounts();
        if accounts
            .iter()
            .any(|account| account.0 == self.account_prefs.active_account)
        {
            return self.account_prefs.active_account.clone();
        }
        let tauri_selected = self.store["activeAccount"].as_str().unwrap_or("");
        if accounts.iter().any(|account| account.0 == tauri_selected) {
            return tauri_selected.to_string();
        }
        accounts
            .first()
            .map(|account| account.0.clone())
            .unwrap_or_default()
    }
    fn refresh_accounts(&mut self) {
        let latest = read_json(launcher_data().join("launcher.json"));
        if latest["accounts"].is_array() {
            self.store["accounts"] = latest["accounts"].clone();
            self.store["activeAccount"] = latest["activeAccount"].clone();
            if let Err(error) = credential_store::protect_legacy_accounts(&mut self.store) {
                self.notice = Some(error);
            }
        }
        match local_accounts::load(&gpui_accounts_path()) {
            Ok(accounts) => self.account_prefs = accounts,
            Err(error) => self.notice = Some(error),
        }
    }
    fn refresh(&mut self) {
        self.instance_refresh_generation += 1;
        self.instances = instances::list(&self.game_dir);
    }
    fn refresh_instances_in_background(&mut self, cx: &mut Context<Self>) {
        self.instance_refresh_generation += 1;
        let generation = self.instance_refresh_generation;
        let dir = self.game_dir.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |view, cx| {
            let instances = executor.spawn(async move { instances::list(&dir) }).await;
            let _ = view.update(cx, |this, cx| {
                if this.instance_refresh_generation == generation {
                    this.instances = instances;
                    cx.notify();
                }
            });
        }).detach();
    }
    fn refresh_skins(&mut self, cx: &mut Context<Self>) {
        if self.skins_loading {
            return;
        }
        if !gpui_store_path().is_file() {
            let latest = read_json(launcher_data().join("launcher.json"));
            if latest["skins"].is_array() {
                self.store["skins"] = latest["skins"].clone();
                self.store["activeSkin"] = latest["activeSkin"].clone();
            }
        }
        let mut entries: Vec<_> = self.store["skins"]
            .as_array()
            .into_iter()
            .flatten()
            .take(127)
            .filter_map(|skin| {
                let id = skin["id"].as_str()?.to_string();
                let file = util::safe_name(skin["file"].as_str()?).ok()?.to_string();
                let slim = skin["model"].as_str() == Some("slim");
                Some((id, launcher_data().join("skins").join(file), slim))
            })
            .collect();
        entries.push((
            "__default_skin".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/assets/game/default_skin.png"),
            false,
        ));
        let executor = cx.background_executor().clone();
        self.skins_loading = true;
        self.skins_checked = true;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let images = executor
                .spawn(async move {
                    entries
                        .into_iter()
                        .filter_map(|(id, path, slim)| {
                            if fs::metadata(&path).ok()?.len() > 256 * 1024 {
                                return None;
                            }
                            let bytes = fs::read(path).ok()?;
                            let preview = skin_preview::front(&bytes, slim).ok()?;
                            Some((
                                id,
                                Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, preview)),
                            ))
                        })
                        .collect::<HashMap<_, _>>()
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.skin_previews = images;
                this.skins_loading = false;
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn refresh_servers(&mut self, cx: &mut Context<Self>) {
        // Once GPUI has its own store, its server list is the one to show; before that, Tauri's.
        if !gpui_store_path().is_file() {
            let latest = read_json(launcher_data().join("launcher.json"));
            if latest["servers"].is_array() {
                self.store["servers"] = latest["servers"].clone();
            }
        }
        if !self.store["servers"].is_array() {
            self.store["servers"] = serde_json::json!([]);
        }
        if let Some(servers) = self.store["servers"].as_array_mut() {
            servers.retain(|server| server["_gpuiLocal"].as_bool() != Some(true));
        }
        match local_servers::load(&gpui_servers_path()) {
            Ok(local) => {
                if let Some(servers) = self.store["servers"].as_array_mut() {
                    for server in local {
                        if let Ok(mut item) = serde_json::to_value(server) {
                            item["_gpuiLocal"] = serde_json::json!(true);
                            servers.push(item);
                        }
                    }
                }
            }
            Err(error) => self.notice = Some(error),
        }
        self.server_checked = true;
        self.server_checks.clear();
        self.refresh_server_pings(cx);
    }

    /// Refresh status in the background without blanking the visible list.
    pub(crate) fn refresh_server_pings(&mut self, cx: &mut Context<Self>) {
        self.server_generation = self.server_generation.wrapping_add(1);
        let generation = self.server_generation;
        let entries: Vec<_> = self.store["servers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|server| {
                Some((
                    server["id"].as_str()?.to_string(),
                    server["address"].as_str()?.to_string(),
                    server["protocol"].as_str().unwrap_or("binary").to_string(),
                ))
            })
            .collect();
        for (id, address, protocol) in entries {
            self.server_checks.entry(id.clone()).or_insert(ServerCheck::Checking);
            let executor = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
                let tcp_only = protocol == "json";
                let result = executor
                    .spawn(async move { server_status::query(&address, &protocol) })
                    .await;
                let _ = view.update(cx, |this, cx| {
                    if this.server_generation != generation {
                        return;
                    }
                    this.server_checks.insert(
                        id,
                        match result {
                            Ok(status) => ServerCheck::Reachable(status, tcp_only),
                            Err(_) => ServerCheck::Failed,
                        },
                    );
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_shell(window, cx)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        ui::theme::load_fonts(cx);
        text_input::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                window_decorations: Some(gpui::WindowDecorations::Client),
                window_min_size: Some(size(px(480.), px(360.))),
                app_id: Some("com.breakmine.launcher".into()),
                ..Default::default()
            },
            |_, cx| cx.new(Launcher::load),
        )
        .expect("Couldn't open GPUI window");
        cx.activate(true);
    });
}

#[cfg(test)]
pub(crate) fn layout_test_launcher(cx: &mut Context<Launcher>) -> Launcher {
    Launcher {
        route: Route::Instances,
        top_open: None,
        dd_open: None,
        dd_closing: None,
        modal: None,
        modal_closing: None,
        toasts: Vec::new(),
        toast_seq: 0,
        remote_skins: HashMap::new(),
        join_target: None,
        minimize_pending: false,
        v: views::ViewState::new(cx),
        store: serde_json::json!({"settings": {"reduceMotion": true}, "servers": [], "accounts": []}),
        game_dir: String::new(),
        selected: String::new(),
        notice: None,
        account_prefs: local_accounts::Accounts::default(),
        game: None,
        game_starting: false,
        offline_task: None,
        server_checks: HashMap::new(),
        server_generation: 0,
        server_checked: true,
        skin_previews: HashMap::new(),
        skins_checked: true,
        skins_loading: false,
        instances: Vec::new(),
        instance_refresh_generation: 0,
    }
}
