//! GPUI-only server entries. The Tauri store remains untouched.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use url::Url;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub address: String,
    pub protocol: String,
}

pub fn load(path: &Path) -> Result<Vec<Server>, String> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Couldn't read GPUI servers: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Couldn't read GPUI servers: {e}")),
    }
}
fn save(path: &Path, servers: &[Server]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid server storage path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".gpui-servers-{}.tmp", crate::util::new_id("save")));
    let result = (|| {
        let bytes = serde_json::to_vec_pretty(servers).map_err(|e| e.to_string())?;
        fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
#[allow(dead_code)]
pub fn add(path: &Path, name: &str, address: &str) -> Result<Server, String> {
    let name = name.trim();
    let address = address.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("Enter a server name of up to 80 characters.".into());
    }
    if address.is_empty() || address.len() > 1024 {
        return Err("Enter a valid server address.".into());
    }
    let url = if address.contains("://") {
        Url::parse(address)
    } else {
        Url::parse(&format!("wss://{address}"))
    }
    .map_err(|_| "Enter a valid server address.")?;
    if !["ws", "wss", "http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use a server address without credentials.".into());
    }
    let mut servers = load(path)?;
    if servers
        .iter()
        .any(|s| s.address.eq_ignore_ascii_case(address))
    {
        return Err("That server is already saved in GPUI.".into());
    }
    let server = Server {
        id: crate::util::new_id("gpui-server"),
        name: name.into(),
        address: address.into(),
        protocol: "binary".into(),
    };
    servers.push(server.clone());
    save(path, &servers)?;
    Ok(server)
}
pub fn remove(path: &Path, id: &str) -> Result<(), String> {
    let mut servers = load(path)?;
    let before = servers.len();
    servers.retain(|server| server.id != id);
    if servers.len() == before {
        return Err("Server not found in GPUI.".into());
    }
    save(path, &servers)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_additions_and_removes_only_requested_entry() {
        let dir = std::env::temp_dir().join(crate::util::new_id("server-test"));
        let path = dir.join("servers.json");
        let first = add(&path, "Alpha", "example.com").unwrap();
        let second = add(&path, "Beta", "wss://example.org:443").unwrap();
        assert!(add(&path, "Again", "EXAMPLE.COM").is_err());
        assert!(add(&path, "Bad", "file:///tmp/a").is_err());
        assert!(add(&path, "Bad", "wss://user:pass@example.com").is_err());
        remove(&path, &first.id).unwrap();
        assert_eq!(load(&path).unwrap()[0].id, second.id);
        assert!(remove(&path, "missing").is_err());
        fs::write(&path, "bad json").unwrap();
        assert!(add(&path, "Gamma", "example.net").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "bad json");
        fs::remove_dir_all(dir).unwrap();
    }
}
