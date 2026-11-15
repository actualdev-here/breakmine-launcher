//! Mod Maker projects stored in the original launcher folder layout:
//! `<appdata>/projects/<id>/project.json` (editor state) and `<appdata>/projects/<id>/mod/`
//! (the generated Patchwork mod, ready to zip or install).
use crate::util::{self, safe_name};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("projects")
}

pub fn dir(data_dir: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(root(data_dir).join(safe_name(id)?))
}

pub fn list(data_dir: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(root(data_dir))
        .map(|rd| rd.flatten().filter_map(|e| fs::read_to_string(e.path().join("project.json")).ok()).filter_map(|s| serde_json::from_str(&s).ok()).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| b["updated"].as_u64().unwrap_or(0).cmp(&a["updated"].as_u64().unwrap_or(0)));
    out
}

/// Only plain relative paths inside the mod folder.
fn rel_path(p: &str) -> Result<PathBuf, String> {
    let path = Path::new(p);
    if p.is_empty() || path.is_absolute() || path.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("Bad file path in project: {p}"));
    }
    Ok(path.to_path_buf())
}

/// Save editor state and regenerate the mod folder from `files`.
pub fn save(data_dir: &Path, project: &Value, files: BTreeMap<String, Vec<u8>>) -> Result<PathBuf, String> {
    let id = project["id"].as_str().ok_or("Project has no id")?;
    let d = dir(data_dir, id)?;
    let staging = d.join(format!(".mod-{}", util::new_id("tmp")));
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let r = (|| -> Result<(), String> {
        for (p, bytes) in files {
            let out = staging.join(rel_path(&p)?);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(out, bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = fs::remove_dir_all(d.join("mod"));
    fs::rename(&staging, d.join("mod")).map_err(|e| e.to_string())?;
    crate::store::write_json(&d.join("project.json"), project)?;
    Ok(d)
}

pub fn zip_mod(data_dir: &Path, id: &str, out: &Path) -> Result<(), String> {
    let src = dir(data_dir, id)?.join("mod");
    if !src.join("ModData.js").is_file() {
        return Err("Save the project first.".into());
    }
    let f = fs::File::create(out).map_err(|e| e.to_string())?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default();
    fn walk(z: &mut zip::ZipWriter<fs::File>, base: &Path, cur: &Path, opts: zip::write::SimpleFileOptions) -> Result<(), String> {
        for e in fs::read_dir(cur).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                walk(z, base, &p, opts)?;
            } else {
                z.start_file(rel, opts).map_err(|e| e.to_string())?;
                z.write_all(&fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    walk(&mut z, &src, &src, opts)?;
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}
