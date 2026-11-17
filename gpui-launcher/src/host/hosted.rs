//! Locally hosted Breakmine servers. World data and runtime are kept separately.
use super::{managed_process::ManagedProcess, shim as store};
use crate::util;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};
use super::shim::{AppHandle, Emitter};

const REVISION: &str = "11db2fe6c2f2105480076bb397fba45710b1df37";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServer {
    pub id: String,
    pub name: String,
    pub port: u16,
    pub world_type: String,
    pub gamemode: String,
    pub seed: String,
    pub motd: String,
    pub allow_mods: bool,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub allow_lan: bool,
}

#[derive(Default)]
pub struct HostState {
    child: Mutex<Option<(String, ManagedProcess)>>,
    last_id: Mutex<Option<String>>,
    pub installing: Mutex<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running_id: Option<String>,
    pub last_id: Option<String>,
    pub runtime_ready: bool,
    pub log: String,
}

fn root(app: &AppHandle) -> PathBuf {
    store::data_dir(app).join("hosted-servers")
}
fn runtime(app: &AppHandle) -> PathBuf {
    root(app).join("runtime")
}
fn worlds(app: &AppHandle) -> PathBuf {
    root(app).join("worlds")
}
fn dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(worlds(app).join(util::safe_name(id)?))
}
pub fn server_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !list(app).iter().any(|s| s.id == id) {
        return Err("Server not found.".into());
    }
    dir(app, id)
}
fn ready(path: &Path) -> bool {
    path.join("server.js").is_file()
        && path.join("node_modules/ws/package.json").is_file()
        && path.join(".launcher-ready").is_file()
}

pub fn list(app: &AppHandle) -> Vec<HostedServer> {
    let mut out: Vec<_> = fs::read_dir(worlds(app))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            fs::read_to_string(e.path().join("launcher-server.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<HostedServer>(&s).ok())
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn save(app: &AppHandle, mut s: HostedServer) -> Result<HostedServer, String> {
    s.name = s.name.trim().to_string();
    if s.name.is_empty() || s.name.len() > 80 {
        return Err("Enter a server name (up to 80 characters).".into());
    }
    if s.port < 1024 {
        return Err("Choose a port from 1024 to 65535.".into());
    }
    if !["flat", "normal", "amplified"].contains(&s.world_type.as_str())
        || !["survival", "creative"].contains(&s.gamemode.as_str())
    {
        return Err("Invalid world or game mode.".into());
    }
    if s.seed.len() > 100 || s.motd.len() > 200 {
        return Err("Seed or description is too long.".into());
    }
    s.owner = s.owner.trim().to_string();
    if s.owner.len() > 32
        || !s
            .owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Owner must be a valid Breakmine username.".into());
    }
    if s.id.is_empty() {
        s.id = util::new_id("server");
    }
    let path = dir(app, &s.id)?;
    if list(app).iter().any(|v| v.id != s.id && v.port == s.port) {
        return Err("Another local server uses that port.".into());
    }
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    store::write_json(&path.join("launcher-server.json"), &s)?;
    Ok(s)
}

pub fn prepare(app: &AppHandle) -> Result<(), String> {
    let progress = |phase: &str, percent: Option<u8>| {
        let _ = app.emit(
            "hosted-install-progress",
            serde_json::json!({ "phase": phase, "percent": percent }),
        );
    };
    progress("Checking requirements", None);
    let base = root(app);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let target = runtime(app);
    if ready(&target) {
        return Ok(());
    }
    let version = Command::new("node")
        .arg("--version")
        .output()
        .map_err(|_| "Install Node.js 22 or newer before installing the server.")?;
    let major = String::from_utf8_lossy(&version.stdout)
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0);
    if !version.status.success() || major < 22 {
        return Err("The server requires Node.js 22 or newer.".into());
    }
    let tmp = base.join(format!("runtime-{}", util::new_id("tmp")));
    fs::create_dir(&tmp).map_err(|e| e.to_string())?;
    let run = |program: &str, args: &[&str], show_git_progress: bool| -> Result<(), String> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!("Couldn't run {program}: {e}. Install Git and Node.js 22 or newer.")
            })?;
        let mut tail = Vec::new();
        let mut line = Vec::new();
        if let Some(stderr) = child.stderr.take() {
            use std::io::BufReader;
            for byte in BufReader::new(stderr).bytes() {
                let Ok(byte) = byte else { break };
                tail.push(byte);
                if tail.len() > 4096 {
                    tail.drain(..1024);
                }
                if byte == b'\r' || byte == b'\n' {
                    if show_git_progress {
                        let text = String::from_utf8_lossy(&line);
                        let phase = if text.contains("Receiving objects") {
                            Some("Downloading server files")
                        } else if text.contains("Resolving deltas") {
                            Some("Processing downloaded files")
                        } else if text.contains("Updating files") {
                            Some("Preparing server files")
                        } else {
                            None
                        };
                        if let Some(phase) = phase {
                            let percent = text
                                .split('%')
                                .next()
                                .and_then(|s| s.split_whitespace().last())
                                .and_then(|s| s.parse::<u8>().ok())
                                .filter(|p| *p <= 100);
                            progress(phase, percent);
                        }
                    }
                    line.clear();
                } else if line.len() < 4096 {
                    line.push(byte);
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            let err = String::from_utf8_lossy(&tail);
            return Err(format!(
                "{program} failed: {}",
                err.chars()
                    .rev()
                    .take(1200)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
        }
        Ok(())
    };
    let result = (|| {
        progress("Preparing download", None);
        run("git", &["init", "--quiet"], false)?;
        progress("Downloading server files", None);
        run(
            "git",
            &[
                "fetch",
                "--progress",
                "--depth",
                "1",
                "https://github.com/Breakmine-Team/breakmine-revived.git",
                REVISION,
            ],
            true,
        )?;
        progress("Preparing server files", None);
        run(
            "git",
            &["checkout", "--progress", "--detach", "FETCH_HEAD"],
            true,
        )?;
        // Include player names in this launcher's server status response.
        let entry = tmp.join("src/js/net/minecraft/server/ServerRuntime.js");
        let original = fs::read_to_string(&entry).map_err(|e| e.to_string())?;
        let needle = "players: getPlayerCount(),";
        if original.matches(needle).count() != 1 {
            return Err("The server runtime doesn't match the expected version.".into());
        }
        let patched = original.replacen(needle, "players: getPlayerCount(),\n                    playerNames: [...getPlayers().values()].map(p => p.username).filter(Boolean),", 1);
        fs::write(entry, patched).map_err(|e| e.to_string())?;
        progress("Installing dependencies", None);
        run(
            if cfg!(windows) { "npm.cmd" } else { "npm" },
            &[
                "ci",
                "--omit=dev",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ],
            false,
        )?;
        progress("Finishing setup", None);
        fs::write(tmp.join(".launcher-ready"), REVISION).map_err(|e| e.to_string())?;
        if target.exists() {
            return Err("An incomplete server runtime already exists. Remove its runtime folder before reinstalling.".into());
        }
        fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    if result.is_ok() {
        progress("Server runtime installed", None);
    }
    result
}

impl HostState {
    pub fn set_icon(&self, app: &AppHandle, id: &str, bytes: &[u8]) -> Result<(), String> {
        let _ = self.status(app);
        let mut lock = self.child.lock().unwrap();
        if let Some((running, child)) = lock.as_mut() {
            if running == id && child.alive() {
                return Err("Stop the server before changing its icon.".into());
            }
        }
        if bytes.len() > 64 * 1024 || util::png_size(bytes) != Some((64, 64)) {
            return Err("The server icon must be a 64×64 PNG under 64 KB.".into());
        }
        let path = server_dir(app, id)?;
        let temp = path.join("server_icon.tmp");
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        fs::rename(temp, path.join("server_icon.png")).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn uninstall_runtime(&self, app: &AppHandle) -> Result<(), String> {
        let _ = self.status(app);
        let _install = self
            .installing
            .try_lock()
            .map_err(|_| "Wait for installation to finish.".to_string())?;
        let mut lock = self.child.lock().unwrap();
        if let Some((_, child)) = lock.as_mut() {
            if child.alive() {
                return Err("Stop the server before uninstalling the runtime.".into());
            }
        }
        *lock = None;
        let path = runtime(app);
        if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|e| format!("Couldn't uninstall the server runtime: {e}"))?;
        }
        Ok(())
    }
    pub fn delete(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let _ = self.status(app);
        let mut lock = self.child.lock().unwrap();
        if let Some((running_id, child)) = lock.as_mut() {
            if running_id == id && child.alive() {
                return Err("Stop the server before deleting it.".into());
            }
        }
        let path = server_dir(app, id)?;
        fs::remove_dir_all(path).map_err(|e| format!("Couldn't delete the server: {e}"))?;
        if lock
            .as_ref()
            .is_some_and(|(running_id, _)| running_id == id)
        {
            *lock = None;
        }
        Ok(())
    }
    pub fn status(&self, app: &AppHandle) -> Status {
        let mut lock = self.child.lock().unwrap();
        if let Some((_, child)) = lock.as_mut() {
            if !child.alive() {
                *lock = None;
            }
        }
        if lock.is_none() {
            let script = runtime(app).join("server.js");
            for server in list(app) {
                let Ok(path) = dir(app, &server.id) else { continue };
                let args = vec!["node".to_string(), script.to_string_lossy().into_owned()];
                if let Some(process) = ManagedProcess::recover(&args, Some(&path)) {
                    *self.last_id.lock().unwrap() = Some(server.id.clone());
                    *lock = Some((server.id, process));
                    break;
                }
            }
        }
        let id = lock.as_ref().map(|(id, _)| id.clone());
        let last_id = self.last_id.lock().unwrap().clone();
        let log = last_id
            .as_deref()
            .and_then(|id| dir(app, id).ok())
            .and_then(|p| fs::File::open(p.join("server.log")).ok())
            .map(|mut f| {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = f.seek(SeekFrom::Start(len.saturating_sub(16000)));
                let mut b = Vec::new();
                let _ = f.take(16000).read_to_end(&mut b);
                String::from_utf8_lossy(&b).into_owned()
            })
            .unwrap_or_default();
        Status {
            running_id: id,
            last_id,
            runtime_ready: ready(&runtime(app)),
            log,
        }
    }
    pub fn start(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let _ = self.status(app);
        let mut lock = self.child.lock().unwrap();
        if let Some((_, child)) = lock.as_mut() {
            if child.alive() {
                return Err("A local server is already running.".into());
            }
        }
        *lock = None;
        let s = list(app)
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("Server not found.")?;
        if !ready(&runtime(app)) {
            return Err("Install the server runtime first.".into());
        }
        let address = std::net::SocketAddr::from((
            if s.allow_lan {
                [0, 0, 0, 0]
            } else {
                [127, 0, 0, 1]
            },
            s.port,
        ));
        let probe = std::net::TcpListener::bind(address).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse { format!("Port {} is already in use. Stop the other server or choose another port in Settings.", s.port) }
            else { format!("Can't use port {}: {e}", s.port) }
        })?;
        drop(probe);
        let path = dir(app, id)?;
        let quote = |v: &str| serde_json::to_string(v).unwrap_or_default();
        let ops = if s.owner.is_empty() {
            String::new()
        } else {
            quote(&s.owner)
        };
        let host = if s.allow_lan { "0.0.0.0" } else { "127.0.0.1" };
        let config = format!("port = {}\nhost = {}\nserver = \"main\"\ndefault_op = false\nop_player_list = [{}]\ndefault_gamemode = {}\nworldType = {}\nseed = {}\nmotd = {}\nallowMods = {}\n", s.port, quote(host), ops, quote(&s.gamemode), quote(&s.world_type), quote(&s.seed), quote(&s.motd), s.allow_mods);
        fs::write(path.join("serverconfig.conf"), config).map_err(|e| e.to_string())?;
        let log = fs::File::create(path.join("server.log")).map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        let mut command = Command::new("node");
        let mut child = command
            .arg(runtime(app).join("server.js"))
            .current_dir(&path)
            .env("DATA_DIR", &path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("Couldn't start Node.js: {e}"))?;
        std::thread::sleep(Duration::from_millis(700));
        if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
            let output = fs::read_to_string(path.join("server.log")).unwrap_or_default();
            if output.contains("EADDRINUSE") {
                return Err(format!("Port {} is already in use. Stop the other server or choose another port in Settings.", s.port));
            }
            let tail: String = output
                .chars()
                .rev()
                .take(800)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            return Err(format!("Server exited ({exit}). {tail}"));
        }
        *self.last_id.lock().unwrap() = Some(id.to_string());
        *lock = Some((id.to_string(), ManagedProcess::Child(child)));
        Ok(())
    }
    pub fn stop(&self) -> Result<(), String> {
        let mut lock = self.child.lock().unwrap();
        let Some((id, mut child)) = lock.take() else {
            return Ok(());
        };
        if let Err(error) = child.stop(Duration::from_secs(10)) {
            *lock = Some((id, child));
            return Err(error);
        }
        Ok(())
    }
}
impl Drop for HostState {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
