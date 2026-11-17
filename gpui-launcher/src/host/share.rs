//! Temporary public access to a running local server via cloudflared.
use super::{hosted, managed_process::ManagedProcess, shim as store};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use super::shim::{AppHandle, Emitter, Manager};

const VERSION: &str = "2026.11.0";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "cloudflared-linux-amd64",
    "51f7c68b57ed7269164075d7e789efce5b59355982f06392deeb037d982a51fd",
));
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ASSET: Option<(&str, &str)> = Some((
    "cloudflared-linux-arm64",
    "abe54a006acf9faf2d19ee9d953cb0d0804e92f59c5382a162e57b84d086f56f",
));
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "cloudflared-windows-amd64.exe",
    "d5c1d82031da6253dcdcff64c944cd0fe063016edeb2e4fd088075fc9bd52d0a",
));
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const ASSET: Option<(&str, &str)> = None;

fn folder(app: &AppHandle) -> PathBuf {
    store::data_dir(app).join("server-sharing")
}
fn binary(app: &AppHandle) -> PathBuf {
    folder(app).join(if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    })
}
struct Tunnel {
    server_id: String,
    child: ManagedProcess,
    log: PathBuf,
    url: Option<String>,
    started: Instant,
}
#[derive(Default)]
pub struct ShareState {
    tunnel: Mutex<Option<Tunnel>>,
    installing: Mutex<()>,
    error: Mutex<Option<String>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareStatus {
    pub installed: bool,
    pub server_id: Option<String>,
    pub url: Option<String>,
    pub error: Option<String>,
}

impl ShareState {
    pub fn install(&self, app: &AppHandle) -> Result<(), String> {
        let _guard = self
            .installing
            .try_lock()
            .map_err(|_| "Sharing installation is already running.")?;
        let (asset, expected) =
            ASSET.ok_or("Automatic installation is not available on this operating system.")?;
        if binary(app).is_file() {
            return Ok(());
        }
        let dir = folder(app);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let temp = dir.join("cloudflared.download");
        let result = (|| {
            let url = format!(
                "https://github.com/cloudflare/cloudflared/releases/download/{VERSION}/{asset}"
            );
            let response = ureq::get(&url)
                .timeout(Duration::from_secs(120))
                .call()
                .map_err(|e| format!("Couldn't download cloudflared: {e}"))?;
            let total = response
                .header("Content-Length")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v <= 100 * 1024 * 1024);
            let mut reader = response.into_reader();
            let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            let mut received = 0u64;
            let mut last_percent = None;
            loop {
                let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                received += n as u64;
                if received > 100 * 1024 * 1024 {
                    return Err("Download is too large.".into());
                }
                file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                hash.update(&buffer[..n]);
                let percent = total.map(|t| (received.saturating_mul(100) / t.max(1)).min(100));
                if percent != last_percent {
                    let _ = app.emit(
                        "share-install-progress",
                        serde_json::json!({"percent": percent}),
                    );
                    last_percent = percent;
                }
            }
            if format!("{:x}", hash.finalize()) != expected {
                return Err("The downloaded sharing tool failed its integrity check.".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&temp, fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
            }
            drop(file);
            fs::rename(&temp, binary(app)).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn start(&self, app: &AppHandle, server_id: &str, port: u16) -> Result<(), String> {
        let _ = self.status(app);
        let mut guard = self.tunnel.lock().unwrap();
        if guard
            .as_mut()
            .is_some_and(|t| t.child.alive())
        {
            return Err("Sharing is already running.".into());
        }
        *guard = None;
        if !binary(app).is_file() {
            return Err("Install the sharing tool first.".into());
        }
        let dir = folder(app);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let logpath = dir.join("tunnel.log");
        let log = fs::File::create(&logpath).map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        let mut command = Command::new(binary(app));
        let child = command
            .args([
                "tunnel",
                "--no-autoupdate",
                "--url",
                &format!("http://127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("Couldn't start sharing: {e}"))?;
        *self.error.lock().unwrap() = None;
        *guard = Some(Tunnel {
            server_id: server_id.into(),
            child: ManagedProcess::Child(child),
            log: logpath,
            url: None,
            started: Instant::now(),
        });
        Ok(())
    }
    pub fn status(&self, app: &AppHandle) -> ShareStatus {
        let host = app.state::<hosted::HostState>().status(app).running_id;
        let mut guard = self.tunnel.lock().unwrap();
        let installed = binary(app).is_file();
        if guard.is_none() {
            if let Some(server) = host.and_then(|id| hosted::list(app).into_iter().find(|s| s.id == id)) {
                let args = vec![binary(app).to_string_lossy().into_owned(), "tunnel".into(), "--no-autoupdate".into(), "--url".into(), format!("http://127.0.0.1:{}", server.port)];
                if let Some(child) = ManagedProcess::recover(&args, None) {
                    *guard = Some(Tunnel { server_id: server.id, child, log: folder(app).join("tunnel.log"), url: None, started: Instant::now() });
                }
            }
        }
        let Some(t) = guard.as_mut() else {
            return ShareStatus {
                installed,
                server_id: None,
                url: None,
                error: self.error.lock().unwrap().clone(),
            };
        };
        if t.url.is_none() {
            let mut contents = String::new();
            if let Ok(file) = fs::File::open(&t.log) {
                let _ = file.take(1024 * 1024).read_to_string(&mut contents);
            }
            t.url = contents
                .split_whitespace()
                .map(|s| {
                    s.trim_matches(|c: char| {
                        !c.is_ascii_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '-'
                    })
                })
                .find(|s| s.starts_with("https://") && s.ends_with(".trycloudflare.com"))
                .map(str::to_string);
        }
        if t.url.is_none() && t.started.elapsed() > Duration::from_secs(60) {
            let _ = t.child.stop(Duration::from_secs(2));
            *guard = None;
            let error = "Couldn't create a public address. Check your connection and try again."
                .to_string();
            *self.error.lock().unwrap() = Some(error.clone());
            return ShareStatus {
                installed,
                server_id: None,
                url: None,
                error: Some(error),
            };
        }
        if !t.child.alive() {
            *guard = None;
            let error = "Sharing stopped unexpectedly. Try again.".to_string();
            *self.error.lock().unwrap() = Some(error.clone());
            return ShareStatus {
                installed,
                server_id: None,
                url: None,
                error: Some(error),
            };
        }
        ShareStatus {
            installed,
            server_id: Some(t.server_id.clone()),
            url: t.url.clone(),
            error: None,
        }
    }
    pub fn stop(&self) {
        let mut guard = self.tunnel.lock().unwrap();
        *self.error.lock().unwrap() = None;
        if let Some(mut t) = guard.take() {
            let _ = t.child.stop(Duration::from_secs(2));
        }
    }
}
impl Drop for ShareState {
    fn drop(&mut self) {
        self.stop();
    }
}
