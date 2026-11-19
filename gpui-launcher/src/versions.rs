//! Older offline builds from the data-store archive.
//! Preserves the legacy `versions.json` index and `versions/<version>/` layout.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs, io::Read, path::{Path, PathBuf}, time::Duration};

const ARCHIVE: &str = "Breakmine-Team/data-store";
const MAX_BUILD: u64 = 256 << 20;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameVersion {
    pub version: String,
    pub commit: String,
    pub date: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(30)).user_agent("Breakmine-Launcher").build()
}

pub fn archive_url(commit: &str, file: &str) -> String {
    format!("https://raw.githubusercontent.com/{ARCHIVE}/{commit}/{file}")
}

pub fn valid(version: &str) -> bool {
    !version.is_empty() && version.len() <= 32 && !version.starts_with('.') && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub fn version_dir(data: &Path, version: &str) -> Result<PathBuf, String> {
    if !valid(version) {
        return Err("Bad game version.".into());
    }
    Ok(data.join("versions").join(version))
}

pub fn installed(data: &Path, version: &str) -> bool {
    version_dir(data, version).map(|d| d.join("index.html").is_file()).unwrap_or(false)
}

/// The cached listing (written by either launcher), so the picker opens instantly.
pub fn cached(data: &Path) -> Vec<GameVersion> {
    fs::read_to_string(data.join("versions.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(data: &Path, list: &[GameVersion]) {
    if let Ok(s) = serde_json::to_string(list) {
        let _ = fs::write(data.join("versions.json"), s);
    }
}

/// Newest first, one entry per version. Reuses the cache for version numbers it already knows.
pub fn refresh(data: &Path) -> Result<Vec<GameVersion>, String> {
    let known: HashMap<String, String> = cached(data).into_iter().map(|v| (v.commit, v.version)).collect();
    let mut out: Vec<GameVersion> = Vec::new();
    for page in 1..=5 {
        let url = format!("https://api.github.com/repos/{ARCHIVE}/commits?path=version.txt&per_page=100&page={page}");
        let list: Value = agent()
            .get(&url)
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(|_| "Can't reach GitHub to list game versions.".to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        let list = list.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            break;
        }
        for c in &list {
            let (Some(sha), Some(date)) = (c["sha"].as_str(), c["commit"]["committer"]["date"].as_str()) else { continue };
            if !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
                continue;
            }
            let version = match known.get(sha) {
                Some(v) => v.clone(),
                None => match agent().get(&archive_url(sha, "version.txt")).call().ok().and_then(|r| r.into_string().ok()) {
                    Some(v) => v.trim().to_string(),
                    None => continue,
                },
            };
            if !valid(&version) || out.iter().any(|g| g.version == version) {
                continue;
            }
            out.push(GameVersion { version, commit: sha.into(), date: date.into() });
        }
        if list.len() < 100 {
            break;
        }
    }
    if out.is_empty() {
        return Err("No game versions found.".into());
    }
    save(data, &out);
    Ok(out)
}

/// Download one archived build into `versions/<version>/` (once; it never auto-updates).
pub fn install(data: &Path, version: &str, mut progress: impl FnMut(u32)) -> Result<PathBuf, String> {
    let dir = version_dir(data, version)?;
    if dir.join("index.html").is_file() {
        return Ok(dir);
    }
    let list = cached(data);
    let entry = match list.iter().find(|v| v.version == version) {
        Some(v) => v.clone(),
        None => refresh(data)?.into_iter().find(|v| v.version == version).ok_or(format!("Breakmine {version} isn't in the archive."))?,
    };
    let resp = agent().get(&archive_url(&entry.commit, "index.html")).call().map_err(|_| format!("Can't download Breakmine {version}."))?;
    let total: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());
    let mut reader = resp.into_reader().take(MAX_BUILD + 1);
    let mut bytes = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(t) = total.filter(|t| *t > 0) {
            progress(((bytes.len() as u64 * 100) / t).min(100) as u32);
        }
    }
    if bytes.len() as u64 > MAX_BUILD {
        return Err("The download is too large.".into());
    }
    if bytes.len() < 1024 || !String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase().contains("<html") {
        return Err("The downloaded game build doesn't look like a web page.".into());
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let part = dir.join("index.html.part");
    fs::write(&part, &bytes).map_err(|e| e.to_string())?;
    fs::rename(&part, dir.join("index.html")).map_err(|e| e.to_string())?;
    fs::write(dir.join("version.txt"), version).map_err(|e| e.to_string())?;
    Ok(dir)
}
