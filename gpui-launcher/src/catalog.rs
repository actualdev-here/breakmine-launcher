//! Listings, installs, and icons from the mods.breakmine.com catalog.
use serde_json::{json, Value};
use std::{io::Read, path::Path, time::Duration};
use url::Url;

const MAX_MOD: u64 = 64 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(30)).user_agent("Breakmine-Launcher").build()
}

pub fn fetch(url: &str) -> Result<Vec<Value>, String> {
    let v: Value = agent().get(url).call().map_err(|_| "Can't reach the mod catalog.".to_string())?.into_json().map_err(|e| e.to_string())?;
    v.get("mods").and_then(|m| m.as_array()).cloned().ok_or_else(|| "The mod catalog answered in an unexpected format.".into())
}

/// Only fetch mods over https. The catalog lists http:// links for its own host, so upgrade those.
pub fn safe_download_url(raw: &str) -> Result<String, String> {
    let mut u = Url::parse(raw).map_err(|_| "Bad download link".to_string())?;
    if u.scheme() == "http" && u.host_str().map(|h| h.ends_with("breakmine.com")).unwrap_or(false) {
        u.set_scheme("https").ok();
    }
    if u.scheme() != "https" {
        return Err("Mods can only be downloaded over https.".into());
    }
    Ok(u.to_string())
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let r = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("Download failed (error {code})."),
        _ => "Download failed: can't reach the server.".into(),
    })?;
    if r.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).is_some_and(|t| t > MAX_MOD) {
        return Err("The download is too large.".into());
    }
    let mut out = Vec::new();
    r.into_reader().take(MAX_MOD + 1).read_to_end(&mut out).map_err(|e| format!("Download interrupted: {e}"))?;
    if out.len() as u64 > MAX_MOD {
        return Err("The download is too large.".into());
    }
    Ok(out)
}

pub fn install(mods_dir: &Path, url: &str, catalog_id: i64, version: &str) -> Result<String, String> {
    let url = safe_download_url(url)?;
    let bytes = download(&url)?;
    crate::instances::install_zip(mods_dir, &bytes, json!({ "source": "catalog", "catalogId": catalog_id, "version": version }))
}

/// A catalog mod's icon.png without installing it, cached by download URL.
pub fn icon(cache_dir: &Path, url: &str) -> Result<Option<Vec<u8>>, String> {
    use std::hash::{Hash, Hasher};
    let url = safe_download_url(url)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hash);
    let cache = cache_dir.join(format!("{:016x}", hash.finish()));
    let bytes = if let Ok(cached) = std::fs::read(&cache) {
        cached
    } else {
        let archive = download(&url)?;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| e.to_string())?;
        let mut icon = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let path = entry.name().trim_end_matches('/').to_string();
            if path.rsplit('/').next() == Some("icon.png") && path.matches('/').count() <= 1 && entry.size() <= 1024 * 1024 {
                entry.read_to_end(&mut icon).map_err(|e| e.to_string())?;
                break;
            }
        }
        if !icon.starts_with(b"\x89PNG\r\n\x1a\n") {
            icon.clear();
        }
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache, &icon);
        icon
    };
    Ok(if bytes.is_empty() { None } else { Some(bytes) })
}
