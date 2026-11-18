//! Temporary mod sharing through the Breakmine Mods staging API.
use std::{fs, io::{Cursor, Read}, path::Path, time::Duration};
const MAX: u64 = 50 * 1024 * 1024;
const BASE: &str = "https://mods.breakmine.com";

pub fn package(mod_dir: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(mod_dir).map_err(|_| "Mod folder not found.")?;
    if !meta.is_dir() || meta.file_type().is_symlink() { return Err("Invalid mod folder.".into()); }
    if !mod_dir.join("ModData.js").is_file() && !mod_dir.join("ModData.json").is_file() { return Err("The mod is missing its metadata file.".into()); }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut pending = vec![mod_dir.to_path_buf()];
    let mut total = 0u64;
    let mut count = 0usize;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() { return Err("Mod folders containing symbolic links cannot be shared.".into()); }
            if ty.is_dir() { pending.push(entry.path()); continue; }
            if !ty.is_file() { continue; }
            let path = entry.path();
            let rel = path.strip_prefix(mod_dir).map_err(|_| "Invalid mod path.")?;
            if rel == Path::new(".from-wiki") { continue; }
            let name = rel.to_str().ok_or("A mod file name is not valid UTF-8.")?.replace('\\', "/");
            count += 1;
            if count > 10000 { return Err("The mod has too many files.".into()); }
            let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let size = file.metadata().map_err(|e| e.to_string())?.len();
            total = total.checked_add(size).ok_or("The mod is too large.")?;
            if total > MAX { return Err("The mod is larger than 50 MB.".into()); }
            zip.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).map_err(|e| e.to_string())?;
            let copied = std::io::copy(&mut file.by_ref().take(MAX - (total - size) + 1), &mut zip).map_err(|e| e.to_string())?;
            if copied != size { return Err("A mod file changed while packaging. Try again.".into()); }
        }
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    if bytes.len() as u64 > MAX { return Err("The packaged mod is larger than 50 MB.".into()); }
    Ok(bytes)
}
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(45)).timeout_write(Duration::from_secs(45)).build()
}
pub fn upload(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX { return Err("Invalid mod package size.".into()); }
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| "Couldn't create a share ID.")?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let boundary = format!("breakmine-{id}");
    let prefix = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"mod.zip\"\r\nContent-Type: application/zip\r\n\r\n");
    let suffix = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(prefix.len() + bytes.len() + suffix.len());
    body.extend_from_slice(prefix.as_bytes()); body.extend_from_slice(bytes); body.extend_from_slice(suffix.as_bytes());
    agent().post(&format!("{BASE}/tempmod/upload/{id}")).set("Content-Type", &format!("multipart/form-data; boundary={boundary}")).send_bytes(&body).map_err(|e| format!("Couldn't share mod: {e}"))?;
    Ok(id)
}
pub fn download(id: &str) -> Result<Vec<u8>, String> {
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') { return Err("Invalid share code.".into()); }
    let response = agent().get(&format!("{BASE}/tempmod/download/{id}")).call().map_err(|e| format!("Couldn't download mod (the link may have expired): {e}"))?;
    let mut bytes = Vec::new();
    response.into_reader().take(MAX + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX { return Err("Invalid mod package size.".into()); }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_mod_can_be_installed_without_launcher_marker() {
        let root = std::env::temp_dir().join(crate::util::new_id("mod-share-test"));
        let source = root.join("source");
        fs::create_dir_all(source.join("assets")).unwrap();
        fs::write(source.join("ModData.json"), r#"{"ID":"sharedtest","NAME":"Shared Test"}"#).unwrap();
        fs::write(source.join("assets/icon.png"), b"test").unwrap();
        fs::write(source.join(".from-wiki"), "private marker").unwrap();
        let bytes = package(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert!(archive.by_name(".from-wiki").is_err());
        assert!(archive.by_name("assets/icon.png").is_ok());
        drop(archive);
        let target = root.join("installed");
        assert_eq!(crate::instances::install_zip(&target, &bytes, serde_json::json!({})).unwrap(), "sharedtest");
        assert_eq!(fs::read(target.join("sharedtest/assets/icon.png")).unwrap(), b"test");
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks() {
        let root = std::env::temp_dir().join(crate::util::new_id("mod-share-test"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ModData.js"), "test").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();
        assert!(package(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_invalid_share_codes_before_network() {
        for code in ["", "../secret", "a/b", "hello world"] { assert!(download(code).is_err()); }
    }
}
