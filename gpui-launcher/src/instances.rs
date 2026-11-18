//! Instances live in `<gameDir>/instances/<id>/`:
//! - `instance.json`  launcher metadata
//! - `mods/<modId>/`   Patchwork mods (ModData.js, ModLoad.js, textures…), fed to the game through `window.modsBridge`
//! - `exports/`        files the game downloads (world .zip exports, structures…)
//! - `profile/`        the game window's own browser storage: worlds, settings, in-game sign-in

use crate::util::{self, safe_name};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const SUBDIRS: [&str; 3] = ["mods", "exports", "profile"];
/// Marker the game uses to auto-enable a mod that lives on disk.
pub const INSTALL_MARKER: &str = ".from-wiki";
const MAX_BRIDGE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub group: String,
    /// "default" follows Settings.build; otherwise "online" | "offline" | "local" | "custom"
    pub build: String,
    /// Pinned offline game version ("" = always the newest). Only used by the offline build.
    pub game_version: String,
    pub notes: String,
    pub pinned: bool,
    pub cover: String,
    /// Pixel icon name from assets/icons ("" = picked from the id).
    pub icon: String,
    pub playtime: u64,
    pub last_played: u64,
    pub created: u64,
    /// Mod ids the launcher keeps out of the game.
    pub disabled_mods: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    #[serde(flatten)]
    pub inst: Instance,
    pub path: String,
    pub cover_path: String,
    pub mods_count: usize,
    pub exports_count: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub icon_path: String,
    /// mods.breakmine.com id when installed from the catalog.
    pub catalog_id: Option<i64>,
    /// Catalog version that was installed (catalog versions don't always match ModData.VERSION).
    pub catalog_version: String,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub file: String,
    pub size: u64,
    pub modified: u64,
    pub path: String,
}

pub fn root(game_dir: &str) -> PathBuf {
    PathBuf::from(game_dir).join("instances")
}

pub fn dir(game_dir: &str, id: &str) -> Result<PathBuf, String> {
    Ok(root(game_dir).join(safe_name(id)?))
}

pub fn read(game_dir: &str, id: &str) -> Result<Instance, String> {
    let p = dir(game_dir, id)?.join("instance.json");
    let s = fs::read_to_string(&p).map_err(|e| format!("Can't read {}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub fn write(game_dir: &str, inst: &Instance) -> Result<(), String> {
    let d = dir(game_dir, &inst.id)?;
    for s in SUBDIRS {
        fs::create_dir_all(d.join(s)).map_err(|e| e.to_string())?;
    }
    crate::store::write_json(&d.join("instance.json"), inst)
}

pub fn view(game_dir: &str, inst: Instance) -> InstanceView {
    let d = dir(game_dir, &inst.id).unwrap_or_default();
    let cover_path = if inst.cover.is_empty() {
        String::new()
    } else {
        d.join(&inst.cover).to_string_lossy().into_owned()
    };
    InstanceView {
        path: d.to_string_lossy().into_owned(),
        cover_path,
        mods_count: list_mods(&d, &inst).iter().filter(|m| m.enabled).count(),
        exports_count: list_exports(&d).len(),
        inst,
    }
}

pub fn list(game_dir: &str) -> Vec<InstanceView> {
    let mut out: Vec<InstanceView> = fs::read_dir(root(game_dir))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| read(game_dir, &e.file_name().to_string_lossy()).ok())
                .map(|i| view(game_dir, i))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        b.inst
            .last_played
            .cmp(&a.inst.last_played)
            .then(a.inst.created.cmp(&b.inst.created))
    });
    out
}

pub fn create(game_dir: &str, name: &str, group: &str) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the instance a name.".into());
    }
    let mut id = util::slug(name);
    while root(game_dir).join(&id).exists() {
        id = format!("{}-{}", util::slug(name), &util::new_id("x")[2..8]);
    }
    let inst = Instance {
        id,
        name: name.into(),
        group: if group.trim().is_empty() {
            "Play".into()
        } else {
            group.trim().into()
        },
        build: "default".into(),
        created: util::now(),
        ..Default::default()
    };
    write(game_dir, &inst)?;
    Ok(inst)
}

/// Create a new instance while reserving its directory atomically.
pub fn create_new(game_dir: &str, name: &str) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the instance a name.".into());
    }
    if name.chars().count() > 80 {
        return Err("The name must be 80 characters or fewer.".into());
    }
    let base = util::slug(name);
    fs::create_dir_all(root(game_dir)).map_err(|e| e.to_string())?;
    let mut attempt = 0;
    let (id, target) = loop {
        let id = if attempt == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, util::new_id("instance"))
        };
        let target = dir(game_dir, &id)?;
        match fs::create_dir(&target) {
            Ok(()) => break (id, target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => return Err(error.to_string()),
        }
    };
    let instance = Instance {
        id,
        name: name.into(),
        group: "Play".into(),
        build: "default".into(),
        created: util::now(),
        ..Default::default()
    };
    if let Err(error) = write(game_dir, &instance) {
        let _ = fs::remove_dir_all(target);
        return Err(error);
    }
    Ok(instance)
}

/// Change one instance field while preserving the latest values of its other fields.
pub fn rename_instance(game_dir: &str, id: &str, name: &str) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the instance a name.".into());
    }
    if name.chars().count() > 80 {
        return Err("The name must be 80 characters or fewer.".into());
    }
    let mut instance = read(game_dir, id)?;
    instance.name = name.into();
    write(game_dir, &instance)?;
    Ok(instance)
}

pub fn set_group(game_dir: &str, id: &str, group: &str) -> Result<Instance, String> {
    let group = group.trim();
    if group.chars().count() > 80 {
        return Err("The folder name must be 80 characters or fewer.".into());
    }
    let mut instance = read(game_dir, id)?;
    instance.group = if group.is_empty() {
        "Play".into()
    } else {
        group.into()
    };
    write(game_dir, &instance)?;
    Ok(instance)
}

pub fn set_build(game_dir: &str, id: &str, build: &str) -> Result<Instance, String> {
    if !["default", "online", "offline", "local", "custom"].contains(&build) {
        return Err("Invalid game build.".into());
    }
    let mut instance = read(game_dir, id)?;
    if build != "offline" {
        instance.game_version.clear();
    }
    instance.build = build.into();
    write(game_dir, &instance)?;
    Ok(instance)
}

pub fn use_newest_version(game_dir: &str, id: &str) -> Result<Instance, String> {
    let mut instance = read(game_dir, id)?;
    instance.build = "offline".into();
    instance.game_version.clear();
    write(game_dir, &instance)?;
    Ok(instance)
}

/// Duplicate an instance and its files into a new independent directory.
pub fn duplicate(game_dir: &str, id: &str) -> Result<Instance, String> {
    let src = read(game_dir, id)?;
    let source = dir(game_dir, id)?;
    let copy_name = format!("{} copy", src.name);
    let base = util::slug(&copy_name);
    fs::create_dir_all(root(game_dir)).map_err(|e| e.to_string())?;
    let mut attempt = 0;
    let (copy_id, target) = loop {
        let copy_id = if attempt == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, util::new_id("copy"))
        };
        let target = dir(game_dir, &copy_id)?;
        match fs::create_dir(&target) {
            Ok(()) => break (copy_id, target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => return Err(error.to_string()),
        }
    };
    let result = (|| {
        util::copy_dir(&source, &target).map_err(|e| e.to_string())?;
        let copy = Instance {
            id: copy_id,
            name: copy_name,
            created: util::now(),
            playtime: 0,
            last_played: 0,
            pinned: false,
            ..src
        };
        write(game_dir, &copy)?;
        Ok(copy)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

/// Import a launcher instance folder into a new directory without modifying the source.
pub fn import_instance(game_dir: &str, source: &Path) -> Result<Instance, String> {
    let source = fs::canonicalize(source).map_err(|e| format!("Couldn't open the folder: {e}"))?;
    let bytes = fs::read(source.join("instance.json"))
        .map_err(|_| "Choose an instance folder containing instance.json.".to_string())?;
    let mut instance: Instance = serde_json::from_slice(&bytes)
        .map_err(|_| "The instance.json file is invalid.".to_string())?;
    let name = if instance.name.trim().is_empty() {
        source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string()
    } else {
        instance.name.trim().to_string()
    };
    fs::create_dir_all(root(game_dir)).map_err(|e| e.to_string())?;
    let destination_root = fs::canonicalize(root(game_dir)).map_err(|e| e.to_string())?;
    if destination_root.starts_with(&source) {
        return Err(
            "Choose an individual instance folder, not the instances directory or its parent."
                .into(),
        );
    }
    let created = create_new(game_dir, &name)?;
    let target = dir(game_dir, &created.id)?;
    let result = (|| {
        copy_import_files(&source, &target)?;
        instance.id = created.id;
        instance.name = name;
        write(game_dir, &instance)?;
        Ok(instance)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn copy_import_files(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let destination = target.join(entry.file_name());
        if kind.is_symlink() {
            return Err("Instance folders containing symbolic links can't be imported.".into());
        }
        if kind.is_dir() {
            copy_import_files(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination).map_err(|e| e.to_string())?;
        } else {
            return Err("The instance contains an unsupported file.".into());
        }
    }
    Ok(())
}

pub fn ensure_default(game_dir: &str) {
    if list(game_dir).is_empty() {
        let _ = create(game_dir, "Survival", "Play");
    }
}

// ── mods ─────────────────────────────────────────────────────────────────

/// Read `static FIELD = "value"` out of a ModData.js class.
fn static_field(src: &str, field: &str) -> Option<String> {
    let mut rest = src;
    while let Some(i) = rest.find("static") {
        rest = &rest[i + 6..];
        let t = rest.trim_start();
        if let Some(after) = t.strip_prefix(field) {
            let after = after.trim_start();
            if let Some(v) = after.strip_prefix('=') {
                let v = v.trim_start();
                let q = v.chars().next()?;
                if q == '"' || q == '\'' || q == '`' {
                    let body = &v[1..];
                    let end = body.find(q)?;
                    return Some(body[..end].to_string());
                }
            }
        }
    }
    None
}

#[derive(Default)]
struct ModMeta {
    id: String,
    name: String,
    author: String,
    version: String,
    description: String,
}

fn parse_mod_data(file_name: &str, src: &str) -> ModMeta {
    if file_name.ends_with(".json") {
        let v: Value = serde_json::from_str(src).unwrap_or(Value::Null);
        let get = |k: &str| {
            v.get(k)
                .or_else(|| v.get(k.to_lowercase()))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        };
        return ModMeta {
            id: get("ID"),
            name: get("NAME"),
            author: get("AUTHOR"),
            version: get("VERSION"),
            description: get("DESCRIPTION"),
        };
    }
    let get = |k: &str| static_field(src, k).unwrap_or_default();
    ModMeta {
        id: get("ID"),
        name: get("NAME"),
        author: get("AUTHOR"),
        version: get("VERSION"),
        description: get("DESCRIPTION"),
    }
}

fn valid_mod_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn read_mod_dir(d: &Path) -> Option<ModMeta> {
    for f in ["ModData.js", "ModData.json"] {
        if let Ok(src) = fs::read_to_string(d.join(f)) {
            return Some(parse_mod_data(f, &src));
        }
    }
    None
}

/// Install a JPH package using the same metadata and patch paths as Patchwork.
pub fn install_jph(mods_dir: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_BRIDGE_BYTES {
        return Err("The mod is larger than 64 MB.".into());
    }
    let data: Value =
        serde_json::from_slice(bytes).map_err(|_| "That file isn't a valid .jph JSON document.")?;
    if data.get("format").and_then(Value::as_str) != Some("jph") {
        return Err("That file isn't a .jph package.".into());
    }
    if data.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("This .jph version isn't supported.".into());
    }
    let id = data
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !valid_mod_id(&id) || !id.as_bytes()[0].is_ascii_alphanumeric() {
        return Err("The .jph has an invalid mod ID.".into());
    }
    let target = mods_dir.join(&id);
    if target.exists() {
        return Err(format!(
            "A mod with ID '{id}' is already installed. Remove it first."
        ));
    }
    let mut patches = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for patch in data
        .get("patches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(source) = patch
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            continue;
        };
        let file = patch
            .get("file")
            .and_then(Value::as_str)
            .ok_or("A patch target is missing.")?
            .replace('\\', "/");
        let mut path = file.trim();
        while let Some(rest) = path.strip_prefix("./") {
            path = rest;
        }
        if path.starts_with('/') && !path.starts_with("/src/js/") {
            return Err("Invalid patch target.".into());
        }
        path = path.strip_prefix('/').unwrap_or(path);
        path = path.strip_prefix("src/js/").unwrap_or(path);
        if !path.ends_with(".js")
            || path.split('/').any(|part| {
                part.is_empty()
                    || part == ".."
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            })
        {
            return Err("Invalid patch target.".into());
        }
        if !seen.insert(path.to_string()) {
            return Err(format!("Duplicate patch target: {path}"));
        }
        patches.push((path.to_string(), source));
    }
    if patches.is_empty() {
        return Err("A .jph must contain at least one patch.".into());
    }
    let field = |key: &str, default: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .unwrap_or(default)
            .trim()
            .to_string()
    };
    let metadata = json!({ "ID": id, "NAME": field("name", &id), "AUTHOR": field("author", "Unknown"), "VERSION": field("modVersion", "1.0.0"), "DESCRIPTION": field("description", ""), "LICENSE": "None", "JPH": 1 });
    fs::create_dir_all(mods_dir).map_err(|e| e.to_string())?;
    let staging = mods_dir.join(format!(".{}-{}", id, util::new_id("tmp")));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        fs::write(staging.join("ModData.json"), metadata.to_string()).map_err(|e| e.to_string())?;
        for (path, source) in patches {
            let dest = staging.join("patches").join(path);
            fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(dest, source).map_err(|e| e.to_string())?;
        }
        fs::write(
            staging.join(INSTALL_MARKER),
            json!({"source":"jph", "installedAt":util::now()}).to_string(),
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&staging, &target).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    Ok(id)
}

pub fn list_mods(inst_dir: &Path, inst: &Instance) -> Vec<ModInfo> {
    let mods_dir = inst_dir.join("mods");
    let mut out: Vec<ModInfo> = fs::read_dir(&mods_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| {
                    let folder = e.file_name().to_string_lossy().into_owned();
                    let meta = read_mod_dir(&e.path())?;
                    let marker: Value = fs::read_to_string(e.path().join(INSTALL_MARKER))
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or(Value::Null);
                    let icon = e.path().join("icon.png");
                    Some(ModInfo {
                        name: if meta.name.is_empty() {
                            folder.clone()
                        } else {
                            meta.name
                        },
                        author: meta.author,
                        version: meta.version,
                        description: meta.description,
                        enabled: !inst.disabled_mods.contains(&folder),
                        icon_path: if icon.is_file() {
                            icon.to_string_lossy().into_owned()
                        } else {
                            String::new()
                        },
                        catalog_id: marker.get("catalogId").and_then(|v| v.as_i64()),
                        catalog_version: marker
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        size: util::dir_size(&e.path()),
                        id: folder,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Update the selected instance's disabled-mod list, preserving its other settings.
pub fn set_mod_enabled(
    game_dir: &str,
    instance_id: &str,
    mod_id: &str,
    enabled: bool,
) -> Result<(), String> {
    safe_name(mod_id)?;
    let mut instance = read(game_dir, instance_id)?;
    let mod_dir = dir(game_dir, instance_id)?.join("mods").join(mod_id);
    if !mod_dir.is_dir() {
        return Err("Mod not found in this instance.".into());
    }
    instance.disabled_mods.retain(|id| id != mod_id);
    if !enabled {
        instance.disabled_mods.push(mod_id.to_string());
    }
    write(game_dir, &instance)
}

/// Copy an unpacked mod without replacing an existing mod or following symlinks.
pub fn install_local_folder(
    game_dir: &str,
    instance_id: &str,
    folder: &Path,
) -> Result<String, String> {
    read(game_dir, instance_id)?;
    let source = fs::canonicalize(folder).map_err(|e| e.to_string())?;
    let meta =
        read_mod_dir(&source).ok_or("That folder has no valid ModData.js or ModData.json.")?;
    if !valid_mod_id(&meta.id) {
        return Err("ModData doesn't declare a usable static ID.".into());
    }
    let mods = dir(game_dir, instance_id)?.join("mods");
    fs::create_dir_all(&mods).map_err(|e| e.to_string())?;
    let mods_canonical = fs::canonicalize(&mods).map_err(|e| e.to_string())?;
    if mods_canonical.starts_with(&source) {
        return Err("Choose the individual mod folder, not its parent.".into());
    }
    let target = mods.join(&meta.id);
    if target.exists() {
        return Err("A mod with this ID is already installed.".into());
    }
    let staging = mods.join(format!(".{}-{}", meta.id, util::new_id("tmp")));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        copy_import_files(&source, &staging)?;
        fs::write(
            staging.join(INSTALL_MARKER),
            json!({"source":"folder", "installedAt":util::now()}).to_string(),
        )
        .map_err(|e| e.to_string())?;
        if target.exists() {
            return Err("A mod with this ID is already installed.".into());
        }
        fs::rename(&staging, &target).map_err(|e| e.to_string())?;
        Ok(meta.id)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

/// Remove only an installed mod belonging to the specified instance.
pub fn remove_mod(game_dir: &str, instance_id: &str, mod_id: &str) -> Result<(), String> {
    safe_name(mod_id)?;
    let mut instance = read(game_dir, instance_id)?;
    let path = dir(game_dir, instance_id)?.join("mods").join(mod_id);
    if !path.is_dir() {
        return Err("Mod not found in this instance.".into());
    }
    fs::remove_dir_all(path).map_err(|e| e.to_string())?;
    if instance.disabled_mods.iter().any(|id| id == mod_id) {
        instance.disabled_mods.retain(|id| id != mod_id);
        write(game_dir, &instance)
            .map_err(|e| format!("Mod removed, but couldn't update its disabled setting: {e}"))?;
    }
    Ok(())
}

/// Install a local mod archive into an existing instance.
pub fn install_archive(
    game_dir: &str,
    instance_id: &str,
    archive: &Path,
) -> Result<String, String> {
    read(game_dir, instance_id)?;
    let ext = archive.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("zip") && !ext.eq_ignore_ascii_case("jph") {
        return Err("Choose a .zip or .jph mod file.".into());
    }
    if fs::metadata(archive).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("Mod file exceeds 64 MB.".into());
    }
    let data = fs::read(archive).map_err(|e| e.to_string())?;
    let mods = dir(game_dir, instance_id)?.join("mods");
    if ext.eq_ignore_ascii_case("jph") {
        install_jph(&mods, &data)
    } else {
        install_zip_inner(&mods, &data, json!({"source": "zip"}), false)
    }
}

/// Install a temporary share without replacing an existing mod.
pub fn install_shared(game_dir: &str, instance_id: &str, data: &[u8]) -> Result<String, String> {
    read(game_dir, instance_id)?;
    let mods = dir(game_dir, instance_id)?.join("mods");
    install_zip_inner(&mods, data, json!({"source": "shared"}), false)
}

/// Unpack a mod .zip into `mods/<ID>/`, where ID comes from its ModData.js.
/// The ModData file may sit at the root or inside one folder.
pub fn install_zip(mods_dir: &Path, bytes: &[u8], marker: Value) -> Result<String, String> {
    install_zip_inner(mods_dir, bytes, marker, true)
}

fn install_zip_inner(
    mods_dir: &Path,
    bytes: &[u8],
    marker: Value,
    replace: bool,
) -> Result<String, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "That file isn't a valid .zip.".to_string())?;
    let mut meta_entry: Option<(String, String)> = None; // (path, file name)
    for i in 0..z.len() {
        let e = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(p) = e.enclosed_name() else { continue };
        let p = p.to_string_lossy().replace('\\', "/");
        if p.starts_with("__MACOSX/") {
            continue;
        }
        let fname = p.rsplit('/').next().unwrap_or("").to_string();
        if fname == "ModData.js" || fname == "ModData.json" {
            let better = match &meta_entry {
                Some((cur, _)) => p.matches('/').count() < cur.matches('/').count(),
                None => true,
            };
            if better {
                meta_entry = Some((p, fname));
            }
        }
    }
    let (meta_path, meta_name) =
        meta_entry.ok_or("ModData.js not found in the archive, so it isn't a Breakmine mod.")?;
    let prefix = meta_path.strip_suffix(&meta_name).unwrap_or("").to_string();
    let mut src = String::new();
    z.by_name(&meta_path)
        .map_err(|e| e.to_string())?
        .read_to_string(&mut src)
        .map_err(|e| e.to_string())?;
    let meta = parse_mod_data(&meta_name, &src);
    if !valid_mod_id(&meta.id) {
        return Err(
            "ModData doesn't declare a usable static ID (letters, numbers, _ or -).".into(),
        );
    }

    let target = mods_dir.join(&meta.id);
    if !replace && target.exists() {
        return Err(
            "A mod with this ID is already installed. Remove it before installing another copy."
                .into(),
        );
    }
    let staging = mods_dir.join(format!(".{}-{}", meta.id, util::new_id("tmp")));
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let mut total = 0u64;
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            if e.is_dir() {
                continue;
            }
            let Some(p) = e.enclosed_name() else { continue };
            let p = p.to_string_lossy().replace('\\', "/");
            let Some(rel) = p.strip_prefix(&prefix) else {
                continue;
            };
            if rel.is_empty() || rel.starts_with("__MACOSX/") {
                continue;
            }
            total += e.size();
            if total > MAX_BRIDGE_BYTES {
                return Err("The mod is larger than 64 MB.".into());
            }
            let out = staging.join(rel);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut buf = Vec::with_capacity(e.size() as usize);
            e.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            fs::write(out, buf).map_err(|e| e.to_string())?;
        }
        let mut m = match marker {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        m.insert("installedAt".into(), json!(util::now()));
        fs::write(staging.join(INSTALL_MARKER), Value::Object(m).to_string())
            .map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    if replace {
        let _ = fs::remove_dir_all(&target);
    }
    if let Err(error) = fs::rename(&staging, &target) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("Couldn't install the mod: {error}"));
    }
    Ok(meta.id)
}

/// Copy an unpacked mod folder (containing ModData.js) into `mods/<ID>/`.
pub fn install_folder(mods_dir: &Path, src: &Path) -> Result<String, String> {
    let meta =
        read_mod_dir(src).ok_or("That folder has no ModData.js, so it isn't a Breakmine mod.")?;
    if !valid_mod_id(&meta.id) {
        return Err("ModData doesn't declare a usable static ID.".into());
    }
    let target = mods_dir.join(&meta.id);
    let _ = fs::remove_dir_all(&target);
    util::copy_dir(src, &target).map_err(|e| e.to_string())?;
    let marker = json!({ "source": "folder", "installedAt": util::now() });
    fs::write(target.join(INSTALL_MARKER), marker.to_string()).map_err(|e| e.to_string())?;
    Ok(meta.id)
}

fn walk(base: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(cur) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue;
        }
        if t.is_dir() {
            walk(base, &e.path(), out);
        } else if t.is_file() {
            if let Ok(rel) = e.path().strip_prefix(base) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

/// Everything `window.modsBridge` serves: relative paths plus file contents.
/// Mirrors the Electron app's preload (text via loadFile, png/ogg via loadBinaryFile).
pub fn bridge_payload(inst_dir: &Path, inst: &Instance) -> Result<Value, String> {
    let mods_dir = inst_dir.join("mods");
    let mut files = Map::new();
    let mut binary = Map::new();
    let mut total = 0u64;
    for m in list_mods(inst_dir, inst).into_iter().filter(|m| m.enabled) {
        let mut paths = vec![];
        walk(&mods_dir, &mods_dir.join(&m.id), &mut paths);
        for rel in paths {
            let key = rel.to_string_lossy().replace('\\', "/");
            let bytes = fs::read(mods_dir.join(&rel)).map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
            if total > MAX_BRIDGE_BYTES {
                return Err(
                    "Enabled mods add up to more than 64 MB. Turn some off and try again.".into(),
                );
            }
            let lower = key.to_lowercase();
            if lower.ends_with(".png") || lower.ends_with(".ogg") {
                binary.insert(
                    key,
                    json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
                );
            } else {
                files.insert(key, json!(String::from_utf8_lossy(&bytes)));
            }
        }
    }
    Ok(json!({ "files": files, "binary": binary }))
}

// ── exports ──────────────────────────────────────────────────────────────

pub fn list_exports(inst_dir: &Path) -> Vec<FileEntry> {
    let mut v: Vec<FileEntry> = fs::read_dir(inst_dir.join("exports"))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| FileEntry {
                    file: e.file_name().to_string_lossy().into_owned(),
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                    modified: util::mtime(&e.path()),
                    path: e.path().to_string_lossy().into_owned(),
                })
                .filter(|f| !f.file.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.modified.cmp(&a.modified));
    v
}

/// A free file name in `dir` for `name` ("world.zip" → "world (2).zip").
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '\0'))
        .collect();
    let name = if name.trim().is_empty() || name == "." || name == ".." {
        "download".to_string()
    } else {
        name
    };
    let p = dir.join(&name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bm-test-{name}-{}", util::new_id("t")));
        fs::create_dir_all(&d).unwrap();
        d
    }

    const MODDATA: &str = "export default class ModData {\n    static NAME = \"What You See\";\n\n    static ID = 'wys';\n    static AUTHOR = `kai`;\n    static VERSION = \"1.0.0\";\n}";

    #[test]
    fn create_new_validates_names_and_keeps_existing_instances() {
        let base = tmp("create-new");
        let game_dir = base.to_string_lossy();
        assert!(create_new(&game_dir, "  ").is_err());
        assert!(create_new(&game_dir, &"a".repeat(81)).is_err());
        let first = create_new(&game_dir, "  My world  ").unwrap();
        assert_eq!(first.name, "My world");
        let first_dir = dir(&game_dir, &first.id).unwrap();
        fs::write(first_dir.join("profile/world.dat"), b"world").unwrap();
        let second = create_new(&game_dir, "My world").unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            fs::read(first_dir.join("profile/world.dat")).unwrap(),
            b"world"
        );
        assert!(dir(&game_dir, &second.id).unwrap().join("mods").is_dir());
        assert_eq!(read(&game_dir, &second.id).unwrap().name, "My world");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn import_preserves_source_and_rejects_unsafe_folders() {
        let base = tmp("import");
        let game = base.join("game");
        let game_dir = game.to_string_lossy();
        let original = create_new(&game_dir, "World").unwrap();
        let source = dir(&game_dir, &original.id).unwrap();
        fs::write(source.join("profile/world.dat"), b"world").unwrap();
        let imported = import_instance(&game_dir, &source).unwrap();
        assert_ne!(original.id, imported.id);
        assert_eq!(
            fs::read(
                dir(&game_dir, &imported.id)
                    .unwrap()
                    .join("profile/world.dat")
            )
            .unwrap(),
            b"world"
        );
        assert_eq!(read(&game_dir, &original.id).unwrap().id, original.id);
        assert!(import_instance(&game_dir, &base).is_err());
        let parent = root(&game_dir);
        fs::write(parent.join("instance.json"), b"{}").unwrap();
        assert!(import_instance(&game_dir, &parent).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&parent, source.join("loop")).unwrap();
            let before = list(&game_dir).len();
            assert!(import_instance(&game_dir, &source).is_err());
            assert_eq!(list(&game_dir).len(), before);
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn instance_settings_preserve_world_and_unrelated_fields() {
        let base = tmp("edit-instance");
        let game_dir = base.to_string_lossy();
        let mut instance = create_new(&game_dir, "World").unwrap();
        instance.disabled_mods.push("demo".into());
        instance.build = "offline".into();
        instance.game_version = "1.2.3".into();
        write(&game_dir, &instance).unwrap();
        let world = dir(&game_dir, &instance.id)
            .unwrap()
            .join("profile/world.dat");
        fs::write(&world, b"world").unwrap();
        assert!(rename_instance(&game_dir, &instance.id, "  ").is_err());
        assert!(set_build(&game_dir, &instance.id, "invalid").is_err());
        rename_instance(&game_dir, &instance.id, " New name ").unwrap();
        set_group(&game_dir, &instance.id, "Folder").unwrap();
        assert_eq!(
            set_build(&game_dir, &instance.id, "offline")
                .unwrap()
                .game_version,
            "1.2.3"
        );
        assert_eq!(
            use_newest_version(&game_dir, &instance.id)
                .unwrap()
                .game_version,
            ""
        );
        set_build(&game_dir, &instance.id, "online").unwrap();
        let saved = read(&game_dir, &instance.id).unwrap();
        assert_eq!(
            (
                saved.name.as_str(),
                saved.group.as_str(),
                saved.build.as_str()
            ),
            ("New name", "Folder", "online")
        );
        assert_eq!(saved.disabled_mods, ["demo"]);
        assert_eq!(fs::read(world).unwrap(), b"world");
        assert!(rename_instance(&game_dir, "../escape", "X").is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn duplicate_preserves_files_and_resets_play_history() {
        let base = tmp("duplicate");
        let game_dir = base.to_string_lossy();
        let mut source = create(&game_dir, "My world", "Play").unwrap();
        source.playtime = 123;
        source.last_played = 42;
        source.pinned = true;
        source.disabled_mods = vec!["demo".into()];
        write(&game_dir, &source).unwrap();
        let src = dir(&game_dir, &source.id).unwrap();
        fs::write(src.join("profile/world.dat"), b"world").unwrap();
        fs::create_dir_all(src.join("mods/demo")).unwrap();
        fs::write(src.join("mods/demo/ModLoad.js"), b"mod").unwrap();
        let copy = duplicate(&game_dir, &source.id).unwrap();
        let dst = dir(&game_dir, &copy.id).unwrap();
        assert_ne!(copy.id, source.id);
        assert_eq!(copy.name, "My world copy");
        assert_eq!(
            (copy.playtime, copy.last_played, copy.pinned),
            (0, 0, false)
        );
        assert_eq!(copy.disabled_mods, ["demo"]);
        assert_eq!(fs::read(dst.join("profile/world.dat")).unwrap(), b"world");
        assert_eq!(fs::read(dst.join("mods/demo/ModLoad.js")).unwrap(), b"mod");
        assert_eq!(read(&game_dir, &copy.id).unwrap().id, copy.id);
        let second = duplicate(&game_dir, &source.id).unwrap();
        assert_ne!(copy.id, second.id);
        assert_eq!(fs::read(dst.join("profile/world.dat")).unwrap(), b"world");
        assert_eq!(read(&game_dir, &source.id).unwrap().playtime, 123);
        assert!(duplicate(&game_dir, "missing").is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn enabling_mod_preserves_instance_settings() {
        let base = tmp("enable");
        let game_dir = base.to_string_lossy();
        let inst = create(&game_dir, "Test", "Play").unwrap();
        let path = dir(&game_dir, &inst.id).unwrap().join("mods/demo");
        fs::create_dir_all(&path).unwrap();
        set_mod_enabled(&game_dir, &inst.id, "demo", false).unwrap();
        let saved = read(&game_dir, &inst.id).unwrap();
        assert_eq!(saved.disabled_mods, ["demo"]);
        assert_eq!(saved.name, "Test");
        set_mod_enabled(&game_dir, &inst.id, "demo", true).unwrap();
        assert!(read(&game_dir, &inst.id).unwrap().disabled_mods.is_empty());
        assert!(set_mod_enabled(&game_dir, &inst.id, "../escape", true).is_err());
        assert!(set_mod_enabled(&game_dir, &inst.id, "missing", true).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn install_archive_validates_instance_and_extension() {
        let base = tmp("archive");
        let game_dir = base.to_string_lossy();
        let inst = create(&game_dir, "Test", "Play").unwrap();
        let package = base.join("demo.jph");
        fs::write(&package, json!({"format":"jph", "version":1, "id":"demo", "patches":[{"file":"src/js/game/Test.js", "source":"x"}]}).to_string()).unwrap();
        assert_eq!(
            install_archive(&game_dir, &inst.id, &package).unwrap(),
            "demo"
        );
        assert!(install_archive(&game_dir, &inst.id, &package).is_err());
        assert!(install_archive(&game_dir, "missing", &package).is_err());
        assert!(install_archive(&game_dir, &inst.id, &base.join("invalid.txt")).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn folder_mod_install_and_remove_are_scoped() {
        let base = tmp("folder-install");
        let game_dir = base.to_string_lossy();
        let instance = create_new(&game_dir, "World").unwrap();
        let source = base.join("mod-source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("ModData.js"), MODDATA).unwrap();
        fs::write(source.join("ModLoad.js"), b"original").unwrap();
        assert_eq!(
            install_local_folder(&game_dir, &instance.id, &source).unwrap(),
            "wys"
        );
        assert!(install_local_folder(&game_dir, &instance.id, &source).is_err());
        assert!(install_local_folder(&game_dir, &instance.id, &base).is_err());
        set_mod_enabled(&game_dir, &instance.id, "wys", false).unwrap();
        assert!(remove_mod(&game_dir, &instance.id, "../outside").is_err());
        remove_mod(&game_dir, &instance.id, "wys").unwrap();
        assert!(read(&game_dir, &instance.id)
            .unwrap()
            .disabled_mods
            .is_empty());
        assert_eq!(fs::read(source.join("ModLoad.js")).unwrap(), b"original");
        assert!(dir(&game_dir, &instance.id)
            .unwrap()
            .join("profile")
            .is_dir());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn archive_import_does_not_replace_installed_mod() {
        let base = tmp("preserve");
        let game_dir = base.to_string_lossy();
        let inst = create(&game_dir, "Test", "Play").unwrap();
        let package = base.join("demo.zip");
        fs::write(
            &package,
            zip_of(&[
                ("ModData.js", MODDATA.as_bytes()),
                ("ModLoad.js", b"original"),
            ]),
        )
        .unwrap();
        assert_eq!(
            install_archive(&game_dir, &inst.id, &package).unwrap(),
            "wys"
        );
        fs::write(
            &package,
            zip_of(&[
                ("ModData.js", MODDATA.as_bytes()),
                ("ModLoad.js", b"replacement"),
            ]),
        )
        .unwrap();
        assert!(install_archive(&game_dir, &inst.id, &package).is_err());
        let installed = dir(&game_dir, &inst.id)
            .unwrap()
            .join("mods/wys/ModLoad.js");
        assert_eq!(fs::read(installed).unwrap(), b"original");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn parses_static_fields() {
        let m = parse_mod_data("ModData.js", MODDATA);
        assert_eq!(
            (
                m.id.as_str(),
                m.name.as_str(),
                m.author.as_str(),
                m.version.as_str()
            ),
            ("wys", "What You See", "kai", "1.0.0")
        );
    }

    #[test]
    fn jph_install_lists_and_bridges_patches() {
        let dir = tmp("jph");
        let mods = dir.join("mods");
        let package = json!({"format":"jph", "version":1, "id":"demo", "name":"Demo Patch", "author":"Kai", "patches":[{"file":"src/js/game/Test.js", "source":"exports.default.test = () => 1;"}]}).to_string();
        assert_eq!(install_jph(&mods, package.as_bytes()).unwrap(), "demo");
        let inst = Instance::default();
        let listed = list_mods(&dir, &inst);
        assert_eq!(listed[0].name, "Demo Patch");
        assert_eq!(listed[0].author, "Kai");
        assert_eq!(
            bridge_payload(&dir, &inst).unwrap()["files"]["demo/patches/game/Test.js"],
            "exports.default.test = () => 1;"
        );
        assert!(install_jph(&mods, package.as_bytes()).is_err());
        let off = Instance {
            disabled_mods: vec!["demo".into()],
            ..inst
        };
        assert!(bridge_payload(&dir, &off).unwrap()["files"]
            .as_object()
            .unwrap()
            .is_empty());
        let bad = json!({"format":"jph", "version":1, "id":"bad", "patches":[{"file":"../escape.js", "source":"x"}]}).to_string();
        assert!(install_jph(&mods, bad.as_bytes()).is_err());
        assert!(!mods.join("bad").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn installs_nested_zip_and_serves_it() {
        let dir = tmp("nested");
        let inst = Instance {
            id: "x".into(),
            ..Default::default()
        };
        let mods = dir.join("mods");
        fs::create_dir_all(&mods).unwrap();
        let z = zip_of(&[
            ("wys-main/ModData.js", MODDATA.as_bytes()),
            ("wys-main/ModLoad.js", b"export default {}"),
            ("wys-main/icon.png", b"\x89PNG..."),
            ("__MACOSX/._x", b"junk"),
        ]);
        assert_eq!(
            install_zip(&mods, &z, json!({"catalogId": 11})).unwrap(),
            "wys"
        );
        assert!(mods.join("wys/ModLoad.js").is_file());
        assert!(mods.join("wys").join(INSTALL_MARKER).is_file());
        let listed = list_mods(&dir, &inst);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].catalog_id, Some(11));
        let b = bridge_payload(&dir, &inst).unwrap();
        assert!(b["files"]["wys/ModData.js"].is_string());
        assert!(b["binary"]["wys/icon.png"].is_string());
        // disabled mods stay out of the game
        let off = Instance {
            disabled_mods: vec!["wys".into()],
            ..inst
        };
        assert!(bridge_payload(&dir, &off).unwrap()["files"]
            .as_object()
            .unwrap()
            .is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_non_mods_and_traversal() {
        let dir = tmp("bad");
        assert!(install_zip(&dir, &zip_of(&[("readme.txt", b"hi")]), Value::Null).is_err());
        let evil = zip_of(&[("ModData.js", MODDATA.as_bytes()), ("../escape.js", b"x")]);
        install_zip(&dir, &evil, Value::Null).unwrap();
        assert!(
            !dir.join("escape.js").exists() && !dir.parent().unwrap().join("escape.js").exists()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// BREAKMINE_MOD_ZIP=/path/to/mod.zip cargo test real_zip -- --ignored
    #[test]
    #[ignore]
    fn real_zip() {
        let p = std::env::var("BREAKMINE_MOD_ZIP").unwrap();
        let dir = tmp("real");
        let id = install_zip(&dir, &fs::read(p).unwrap(), Value::Null).unwrap();
        println!(
            "installed {id}: {:?}",
            fs::read_dir(dir.join(&id))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<Vec<_>>()
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
