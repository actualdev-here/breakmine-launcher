//! Download the latest offline build without changing the Tauri copy.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use url::Url;

const LIMIT: u64 = 512 * 1024 * 1024;
#[derive(Default, Clone)]
pub struct Progress {
    pub received: u64,
    pub total: Option<u64>,
    pub phase: &'static str,
}
#[derive(Serialize, Deserialize)]
struct Marker {
    version: String,
    directory: String,
}

pub fn current(data: &Path) -> Option<(String, PathBuf)> {
    let root = data.join("gpui-offline");
    let marker: Marker = serde_json::from_slice(&fs::read(root.join("current.json")).ok()?).ok()?;
    if !valid_version(&marker.version)
        || !marker.directory.starts_with("release-")
        || !marker
            .directory
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let root = fs::canonicalize(root).ok()?;
    let dir = root.join(marker.directory);
    if fs::symlink_metadata(&dir).ok()?.file_type().is_symlink() {
        return None;
    }
    let file = fs::canonicalize(dir.join("index.html")).ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    Some((marker.version, file))
}
fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && !v.starts_with('.')
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}
fn canceled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Download canceled.".into())
    } else {
        Ok(())
    }
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn download(
    source: &str,
    data: &Path,
    cancel: &AtomicBool,
    progress: &Arc<Mutex<Progress>>,
) -> Result<String, String> {
    download_inner(source, data, cancel, progress, false)
}
fn download_inner(
    source: &str,
    data: &Path,
    cancel: &AtomicBool,
    progress: &Arc<Mutex<Progress>>,
    allow_local_http: bool,
) -> Result<String, String> {
    canceled(cancel)?;
    let mut base = Url::parse(source).map_err(|_| "Invalid offline source URL.")?;
    let local_http = allow_local_http
        && base.scheme() == "http"
        && matches!(base.host_str(), Some("127.0.0.1") | Some("localhost"));
    if (base.scheme() != "https" && !local_http)
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("Offline source must be an HTTPS URL.".into());
    }
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(10))
        .build();
    let get = |name: &str| -> Result<ureq::Response, String> {
        let url = base.join(name).map_err(|_| "Invalid offline source URL.")?;
        agent
            .get(url.as_str())
            .call()
            .map_err(|e| format!("Couldn't download {name}: {e}"))
    };
    let mut version_data = Vec::new();
    get("version.txt")?
        .into_reader()
        .take(129)
        .read_to_end(&mut version_data)
        .map_err(|e| e.to_string())?;
    let version = std::str::from_utf8(&version_data)
        .map_err(|_| "Invalid offline version.")?
        .trim();
    if !valid_version(version) {
        return Err("Invalid offline version.".into());
    }
    canceled(cancel)?;
    let root = data.join("gpui-offline");
    fs::create_dir_all(&root).map_err(|e| format!("Couldn't create download directory: {e}"))?;
    let mut random = [0u8; 12];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let stage = root.join(format!("stage-{id}"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let mut cleanup = Cleanup(stage.clone());
    let response = get("index.html")?;
    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|n| n > LIMIT) {
        return Err("Offline build is too large.".into());
    }
    let check_length = response.header("Content-Encoding").is_none();
    if let Ok(mut p) = progress.lock() {
        p.phase = "Downloading";
        p.total = total;
    }
    let mut input = response.into_reader();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join("index.html"))
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 65536];
    let mut first = Vec::new();
    let mut received = 0u64;
    loop {
        canceled(cancel)?;
        let n = input
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > LIMIT {
            return Err("Offline build is too large.".into());
        }
        if first.len() < 4096 {
            first.extend_from_slice(&buf[..n.min(4096 - first.len())]);
        }
        output
            .write_all(&buf[..n])
            .map_err(|e| format!("Couldn't save offline build: {e}"))?;
        if let Ok(mut p) = progress.lock() {
            p.received = received;
        }
    }
    if received < 1024
        || (check_length && total.is_some_and(|n| n != received))
        || !String::from_utf8_lossy(&first)
            .to_ascii_lowercase()
            .contains("<html")
    {
        return Err("Downloaded offline build is incomplete or invalid.".into());
    }
    output.sync_all().map_err(|e| e.to_string())?;
    canceled(cancel)?;
    let directory = format!("release-{id}");
    let release = root.join(&directory);
    fs::rename(&stage, &release).map_err(|e| e.to_string())?;
    cleanup.0 = release;
    let marker = serde_json::to_vec(&Marker {
        version: version.into(),
        directory,
    })
    .map_err(|e| e.to_string())?;
    let temp = root.join(format!("marker-{id}.tmp"));
    let result = (|| -> Result<(), String> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        f.write_all(&marker).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        canceled(cancel)?;
        fs::rename(&temp, root.join("current.json")).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result?;
    std::mem::forget(cleanup);
    Ok(version.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn fixture(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for payload in [b"1.2.3\n".to_vec(), body] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                )
                .unwrap();
                stream.write_all(&payload).unwrap();
            }
        });
        (format!("http://{addr}/"), handle)
    }
    #[test]
    fn download_preserves_existing_build_on_failure_and_cancel() {
        let dir = std::env::temp_dir().join(format!(
            "gpui-offline-test-{}-{}",
            std::process::id(),
            crate::util::now()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("offline")).unwrap();
        fs::write(dir.join("offline/index.html"), "tauri").unwrap();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = AtomicBool::new(false);
        let (source, server) = fixture(format!("<html>{}</html>", "x".repeat(2048)).into_bytes());
        assert_eq!(
            download_inner(&source, &dir, &cancel, &progress, true).unwrap(),
            "1.2.3"
        );
        server.join().unwrap();
        let (_, original) = current(&dir).unwrap();
        assert!(fs::read_to_string(&original).unwrap().starts_with("<html>"));
        assert_eq!(
            fs::read_to_string(dir.join("offline/index.html")).unwrap(),
            "tauri"
        );
        let (source, server) = fixture(vec![b'x'; 2048]);
        assert!(download_inner(&source, &dir, &cancel, &progress, true).is_err());
        server.join().unwrap();
        assert_eq!(current(&dir).unwrap().1, original);
        cancel.store(true, Ordering::Relaxed);
        assert!(download_inner(&source, &dir, &cancel, &progress, true).is_err());
        assert_eq!(current(&dir).unwrap().1, original);
        assert!(fs::read_dir(dir.join("gpui-offline"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("stage-")));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_insecure_sources_and_traversal_marker() {
        let dir = std::env::temp_dir().join(format!("gpui-offline-invalid-{}", std::process::id()));
        fs::create_dir_all(dir.join("gpui-offline")).unwrap();
        fs::write(
            dir.join("gpui-offline/current.json"),
            r#"{"version":"1","directory":"release-../../offline"}"#,
        )
        .unwrap();
        assert!(current(&dir).is_none());
        assert!(download(
            "http://example.com",
            &dir,
            &AtomicBool::new(false),
            &Arc::new(Mutex::new(Progress::default()))
        )
        .is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

/// The newest published offline version (`version.txt` at the source), for the Settings status line.
pub fn latest(source: &str) -> Result<String, String> {
    let mut base = Url::parse(source).map_err(|_| "Invalid offline source URL.")?;
    if base.scheme() != "https" {
        return Err("Offline source must be an HTTPS URL.".into());
    }
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let url = base.join("version.txt").map_err(|_| "Invalid offline source URL.")?;
    let text = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(8))
        .build()
        .get(url.as_str())
        .call()
        .map_err(|_| "Can't reach the offline build source.".to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v = text.trim().to_string();
    if valid_version(&v) { Ok(v) } else { Err("The offline source sent an unexpected version.".into()) }
}
