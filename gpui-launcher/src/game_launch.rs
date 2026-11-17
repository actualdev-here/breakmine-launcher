//! Build a game launch configuration without exposing credentials in command-line arguments.
use crate::{instances, local_accounts};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use url::Url;

#[derive(Serialize)]
pub struct HostConfig {
    url: String,
    title: String,
    profile: PathBuf,
    exports: PathBuf,
    init_script: String,
    width: u32,
    height: u32,
}

pub fn prepare(
    game_dir: &str,
    data_dir: &Path,
    id: &str,
    selected_account: &str,
    store: &Value,
    prefs: &local_accounts::Accounts,
) -> Result<HostConfig, String> {
    let instance = instances::read(game_dir, id)?;
    let dir = fs::canonicalize(instances::dir(game_dir, id)?).map_err(|e| e.to_string())?;
    let settings = &store["settings"];
    let build = if instance.build.is_empty() || instance.build == "default" {
        settings["build"].as_str().unwrap_or("online")
    } else {
        &instance.build
    };
    let mut url = match build {
        "offline" => {
            let path = if instance.game_version.is_empty() {
                crate::offline_download::current(data_dir)
                    .map(|(_, path)| path)
                    .unwrap_or_else(|| data_dir.join("offline/index.html"))
            } else {
                let version = &instance.game_version;
                if version.len() > 32
                    || version.starts_with('.')
                    || !version
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                {
                    return Err("Invalid game version.".into());
                }
                data_dir.join("versions").join(version).join("index.html")
            };
            if !path.is_file() {
                return Err(
                    "This offline build isn't downloaded yet. Download it in GPUI Settings first."
                        .into(),
                );
            }
            Url::from_file_path(path).map_err(|_| "Invalid offline build path.")?
        }
        "local" => Url::parse(
            settings["localUrl"]
                .as_str()
                .unwrap_or("http://localhost:8000/"),
        )
        .map_err(|_| "Invalid local game URL.")?,
        "custom" => Url::parse(settings["customUrl"].as_str().unwrap_or(""))
            .map_err(|_| "Set a valid custom game URL in Settings.")?,
        "online" | "default" | "" => Url::parse(
            settings["onlineUrl"]
                .as_str()
                .unwrap_or("https://game.breakmine.com/"),
        )
        .map_err(|_| "Invalid online game URL.")?,
        _ => return Err("Invalid game build.".into()),
    };
    if !["https", "http", "file"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Invalid game URL.".into());
    }
    let mut account = None;
    if let Some(local) = prefs
        .online_profiles
        .iter()
        .find(|a| a.id == selected_account)
    {
        if local.token.is_empty() || local.expires <= crate::util::now() + 60 {
            return Err(format!(
                "{}'s session expired. Connect the account again.",
                local.name
            ));
        }
        account = Some(json!({"username": local.name, "token": local.token}));
    } else if prefs
        .offline_profiles
        .iter()
        .any(|a| a.id == selected_account)
    {
        // Offline profiles do not carry an account token.
    } else if let Some(saved) = store["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"].as_str() == Some(selected_account))
    {
        if saved["offline"].as_bool() != Some(true) {
            let token = saved["token"].as_str().unwrap_or("");
            let name = saved["name"].as_str().unwrap_or("Account");
            let expires = saved["expires"].as_u64().unwrap_or(0);
            if token.is_empty() || expires <= crate::util::now() + 60 {
                return Err(format!(
                    "{name}'s session expired. Connect the account again."
                ));
            }
            account = Some(json!({"username": name, "token": token}));
        }
    } else {
        return Err("Select an account before starting the game.".into());
    }
    if account.is_some()
        && url.scheme() == "http"
        && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err("An online account requires HTTPS or a local game URL.".into());
    }
    let others: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "launcher")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(others)
        .append_pair("launcher", "1");
    let servers: Vec<_> = store["servers"].as_array().into_iter().flatten().map(|server| json!({"name": server["name"], "address": server["address"], "motd": server["motd"], "protocol": if server["protocol"].as_str() == Some("json") {"json"} else {"binary"}})).collect();
    let bridge = instances::bridge_payload(&dir, &instance)?;
    // `__join` (a server id) is set by the launcher when Join was pressed: the game connects to it once booted.
    let join = store["__join"].as_str().and_then(|id| store["servers"].as_array()?.iter().find(|s| s["id"].as_str() == Some(id)).map(|s| json!({"name": s["name"], "address": s["address"]})));
    let config = json!({"launcherVersion": env!("CARGO_PKG_VERSION"), "account": account, "servers": servers, "join": join, "mods": bridge});
    let serialized = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    let init_script = format!(
        "(function(C){{{}}})({});",
        include_str!("game_init.js"),
        serialized
    );
    let profile = dir.join("profile-gpui");
    let exports = dir.join("exports");
    // This profile is separate to avoid another launcher using the same WebKit storage at once.
    if fs::symlink_metadata(&profile).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("The GPUI game profile cannot be a symbolic link.".into());
    }
    fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    fs::create_dir_all(&exports).map_err(|e| e.to_string())?;
    let profile = fs::canonicalize(profile).map_err(|e| e.to_string())?;
    let exports = fs::canonicalize(exports).map_err(|e| e.to_string())?;
    if !profile.starts_with(&dir) || !exports.starts_with(&dir) {
        return Err("Game profile and exports must be inside the instance folder.".into());
    }
    Ok(HostConfig {
        url: url.to_string(),
        title: format!("Breakmine — {}", instance.name),
        profile,
        exports,
        init_script,
        width: settings["windowWidth"]
            .as_u64()
            .unwrap_or(1280)
            .clamp(640, 7680) as u32,
        height: settings["windowHeight"]
            .as_u64()
            .unwrap_or(720)
            .clamp(400, 4320) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepare_isolates_profiles_and_rejects_insecure_account_url() {
        let base = std::env::temp_dir().join(crate::util::new_id("game-plan-test"));
        fs::create_dir_all(&base).unwrap();
        let game = base.join("game");
        let game_dir = game.to_str().unwrap();
        let inst = instances::create_new(game_dir, "Test").unwrap();
        let mut prefs = local_accounts::Accounts::default();
        prefs.online_profiles.push(local_accounts::OnlineProfile {
            id: "id".into(),
            name: "Alex".into(),
            token: "secret-test-token".into(),
            expires: crate::util::now() + 3600,
            api: "https://api.example.com".into(),
        });
        let mut store = json!({"settings":{"build":"online","onlineUrl":"https://game.example/play?x=1"},"servers":[]});
        let plan = prepare(game_dir, &base, &inst.id, "id", &store, &prefs).unwrap();
        assert!(plan.url.contains("launcher=1"));
        assert!(!plan.url.contains("secret-test-token"));
        assert!(plan.profile.ends_with("profile-gpui"));
        assert!(plan.init_script.contains("secret-test-token"));
        #[cfg(unix)]
        {
            let instance_dir = instances::dir(game_dir, &inst.id).unwrap();
            fs::remove_dir(instance_dir.join("profile-gpui")).unwrap();
            std::os::unix::fs::symlink(
                instance_dir.join("profile"),
                instance_dir.join("profile-gpui"),
            )
            .unwrap();
            assert!(prepare(game_dir, &base, &inst.id, "id", &store, &prefs).is_err());
        }
        store["settings"]["onlineUrl"] = json!("http://game.example/");
        assert!(prepare(game_dir, &base, &inst.id, "id", &store, &prefs).is_err());
        assert!(prepare(game_dir, &base, &inst.id, "missing", &store, &prefs).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}

pub struct StartedGame {
    pub child: std::process::Child,
    pub status: std::sync::Arc<std::sync::Mutex<Option<(crate::telemetry::Snapshot, std::time::Instant)>>>,
}

/// Start the game host and wait until its WebView has been created.
pub fn start(config: HostConfig) -> Result<StartedGame, String> {
    let executable = if cfg!(windows) {
        "breakmine-game-host.exe"
    } else {
        "breakmine-game-host"
    };
    let beside_launcher = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(executable)));
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("game-host/target/debug")
        .join(executable);
    let host = beside_launcher
        .filter(|p| p.is_file())
        .or_else(|| development.is_file().then_some(development))
        .ok_or("Game host is missing. Build gpui-launcher/game-host first.")?;
    start_with_host(config, &host)
}

fn start_with_host(config: HostConfig, host: &Path) -> Result<StartedGame, String> {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    let serialized = serde_json::to_vec(&config).map_err(|_| "Couldn't prepare game settings.")?;
    if serialized.len() >= 100 * 1024 * 1024 {
        return Err("Game settings are too large. Disable some mods and try again.".into());
    }
    let mut child = Command::new(host)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start game window: {e}"))?;
    let write_result = child
        .stdin
        .as_mut()
        .ok_or("Couldn't open game connection.")
        .and_then(|input| {
            input
                .write_all(&serialized)
                .and_then(|_| input.write_all(b"\n"))
                .and_then(|_| input.flush())
                .map_err(|_| "Couldn't send game settings.")
        });
    drop(serialized);
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let output = child.stdout.take().ok_or("Couldn't check game startup.")?;
    let (tx, rx) = mpsc::sync_channel(1);
    let status = std::sync::Arc::new(std::sync::Mutex::new(None));
    let status_sink = status.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        let mut ready = false;
        for line in (&mut reader).lines().take(128) {
            if matches!(line, Ok(ref s) if s == "READY") {
                let _ = tx.send(true);
                ready = true;
                break;
            }
        }
        if ready {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Some(snapshot) = line.strip_prefix("STATUS ").and_then(crate::telemetry::Snapshot::parse) {
                    if let Ok(mut target) = status_sink.lock() { *target = Some((snapshot, std::time::Instant::now())); }
                }
            }
        } else {
            let _ = tx.send(false);
        }
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(true) => Ok(StartedGame { child, status }),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Err("Couldn't open the game window. Check WebView and graphics support.".into())
        }
    }
}

#[cfg(all(test, unix))]
mod host_tests {
    use super::*;
    use std::{io::Write, os::unix::fs::PermissionsExt};

    #[test]
    fn host_receives_settings_over_stdin_and_stops_gracefully() {
        let base = std::env::temp_dir().join(crate::util::new_id("game-host-test"));
        fs::create_dir_all(&base).unwrap();
        let script = base.join("host.sh");
        let args_file = base.join("args-count");
        let contents = format!(
            "#!/bin/sh\nread -r config\nprintf '%s' \"$#\" > '{}'\nprintf 'READY\\n'\nprintf '%s\\n' 'STATUS {{\"state\":\"playing\",\"connected\":true,\"players\":[\"Alex\"]}}'\nread -r command\ntest \"$command\" = stop\n",
            args_file.display()
        );
        fs::write(&script, contents).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let config = HostConfig {
            url: "https://game.example".into(),
            title: "Test".into(),
            profile: base.clone(),
            exports: base.clone(),
            init_script: "secret-token".into(),
            width: 640,
            height: 400,
        };
        let mut started = start_with_host(config, &script).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while started.status.lock().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(started.status.lock().unwrap().as_ref().unwrap().0.players, ["Alex"]);
        let child = &mut started.child;
        assert_eq!(fs::read_to_string(&args_file).unwrap(), "0");
        child.stdin.as_mut().unwrap().write_all(b"stop\n").unwrap();
        assert!(child.wait().unwrap().success());
        fs::remove_dir_all(base).unwrap();
    }
}
