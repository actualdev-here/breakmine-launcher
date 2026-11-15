//! Turns a Mod Maker project into Patchwork mod files (port of src/js/creator/generate.js).
//!
//! Projects are the same JSON the web Mod Maker saves, so both launchers can open them:
//!   blocks/Block<Name>.js → id <mod>:<name_in_snake_case>, items/Item<Name>.js likewise,
//!   textures/<key>.png referenced as '<mod>:<key>', crafting/Block<Name>Crafting.js targets <mod>:<name>.
use base64::Engine;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub const SOUNDS: [&str; 8] = ["stone", "wood", "grass", "gravel", "sand", "glass", "cloth", "leaves"];

pub fn pascal(key: &str) -> String {
    key.split('_').filter(|w| !w.is_empty()).map(|w| {
        let mut c = w.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }).collect()
}

pub fn to_key(name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(ch);
        } else {
            gap = true;
        }
    }
    out.chars().take(40).collect()
}

pub fn valid_key(k: &str) -> bool {
    let mut c = k.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase()) && k.len() <= 40 && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

/// The id the game derives from the generated class name: `ruby_block_2` loads as `ruby_block2`.
pub fn game_id(key: &str) -> String {
    let p = pascal(key);
    let mut out = String::new();
    let chars: Vec<char> = p.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit()) {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

pub fn valid_mod_id(k: &str) -> bool {
    let mut c = k.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase()) && (2..=32).contains(&k.len()) && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

pub fn patch_path(path: &str) -> Option<String> {
    let mut p = path.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    if p.starts_with('/') && !p.starts_with("/src/js/") {
        return None;
    }
    let p = p.trim_start_matches('/');
    let p = p.strip_prefix("src/js/").unwrap_or(p).to_string();
    let ok = p.ends_with(".js") && p.split('/').all(|part| !part.is_empty() && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')));
    ok.then_some(p)
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
fn str_lit(v: &Value) -> String {
    serde_json::to_string(&s(v)).unwrap_or_else(|_| "\"\"".into())
}
fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}
fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.),
        Value::String(s) => s.parse().unwrap_or(0.),
        Value::Bool(b) => *b as i32 as f64,
        _ => 0.,
    }
}
fn indent(code: &str, pad: &str) -> String {
    code.split('\n').map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }).collect::<Vec<_>>().join("\n")
}

fn mod_data(p: &Value) -> String {
    let version = if s(&p["version"]).is_empty() { "1.0.0".to_string() } else { s(&p["version"]) };
    let license = if s(&p["license"]).is_empty() { "All rights reserved".to_string() } else { s(&p["license"]) };
    format!(
        "export default class ModData {{\n    static NAME = {};\n\n    static ID = {};\n\n    static AUTHOR = {};\n\n    static VERSION = {};\n\n    static DESCRIPTION = {};\n\n    static LICENSE = {};\n}}\n",
        str_lit(&p["name"]),
        str_lit(&p["modId"]),
        str_lit(&p["author"]),
        serde_json::to_string(&version).unwrap(),
        str_lit(&p["description"]),
        serde_json::to_string(&license).unwrap()
    )
}

fn mod_load(p: &Value) -> String {
    let code = s(&p["code"]);
    let body = if code.trim().is_empty() { format!("console.log('[{}] loaded');", s(&p["modId"])) } else { code.trim().to_string() };
    format!(
        "export default class ModLoad {{\n    // Runs when the mod loads. The third argument is the ModLoader: anything registered\n    // through it (commands, render hooks, keybinds) is removed again when the mod is turned off.\n    static onLoad(world, minecraft, modLoader) {{\n{}\n    }}\n}}\n",
        indent(&body, "        ")
    )
}

fn block(p: &Value, b: &Value) -> String {
    let key = s(&b["key"]);
    let cls = format!("Block{}", pascal(&key));
    let sound = if s(&b["sound"]).is_empty() { "stone".to_string() } else { s(&b["sound"]) };
    let hardness = if b["unbreakable"].as_bool().unwrap_or(false) { "-1.0".to_string() } else { format!("{:.1}", if b["hardness"].is_null() { 1. } else { num(&b["hardness"]) }) };
    let lines = [
        format!("        this.description = {};", str_lit(&b["name"])),
        format!("        this.hardness = {hardness};"),
        format!("        if (Block.sounds && Block.sounds.{sound}) this.sound = Block.sounds.{sound};"),
    ];
    let mut extra = Vec::new();
    if num(&b["light"]) > 0. {
        extra.push(format!("    getLightValue(world, x, y, z) {{\n        return {};\n    }}", num(&b["light"]).round().clamp(0., 15.) as i64));
    }
    if b["transparent"].as_bool().unwrap_or(false) {
        extra.push("    isTranslucent() {\n        return true;\n    }".to_string());
        extra.push("    occludesNeighborFaces() {\n        return false;\n    }".to_string());
    }
    let m = s(&p["modId"]);
    format!(
        "import Block from \"../Block.js\";\n\n// Registered as {m}:{}\nexport default class {cls} extends Block {{\n    static inventoryTab = {};\n\n    constructor(id, textureSlotId) {{\n        super(id, textureSlotId);\n{}\n    }}\n\n    getTextureForFace(face) {{\n        return '{m}:{key}';\n    }}\n{}}}\n",
        game_id(&key),
        str_lit(&p["name"]),
        lines.join("\n"),
        if extra.is_empty() { String::new() } else { format!("\n{}\n", extra.join("\n\n")) }
    )
}

fn item(p: &Value, it: &Value) -> String {
    let key = s(&it["key"]);
    let m = s(&p["modId"]);
    format!(
        "import Item from \"../Item.js\";\n\n// Registered as {m}:{}\nexport default class Item{} extends Item {{\n    static inventoryTab = {};\n\n    constructor(id, textureSlotId) {{\n        super(id, textureSlotId);\n        this.tex = '{m}:{key}';\n        this.description = {};\n    }}\n\n    getTextureForFace(face) {{\n        return '{m}:{key}';\n    }}\n}}\n",
        game_id(&key),
        pascal(&key),
        str_lit(&p["name"]),
        str_lit(&it["name"])
    )
}

pub struct Shape {
    pub width: usize,
    pub height: usize,
    pub list: Vec<i64>,
}

/// Trim the 3×3 grid to the used rectangle (shaped) or list the ingredients (shapeless).
pub fn recipe_shape(r: &Value) -> Shape {
    let g: Vec<i64> = arr(&r["grid"]).iter().map(|v| num(v) as i64).collect();
    if r["shapeless"].as_bool().unwrap_or(false) {
        return Shape { width: 0, height: 0, list: g.into_iter().filter(|v| *v != 0).collect() };
    }
    let (mut x0, mut y0, mut x1, mut y1) = (3i32, 3i32, -1i32, -1i32);
    for (i, v) in g.iter().enumerate() {
        if *v == 0 {
            continue;
        }
        let (x, y) = ((i % 3) as i32, (i / 3) as i32);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if x1 < 0 {
        return Shape { width: 0, height: 0, list: Vec::new() };
    }
    let mut list = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            list.push(g.get((y * 3 + x) as usize).copied().unwrap_or(0));
        }
    }
    Shape { width: (x1 - x0 + 1) as usize, height: (y1 - y0 + 1) as usize, list }
}

fn crafting(r: &Value) -> String {
    let sh = recipe_shape(r);
    let shapeless = r["shapeless"].as_bool().unwrap_or(false);
    let count = num(&r["count"]);
    let count_txt = if count == 0. { "1".to_string() } else { s(&r["count"]) };
    format!(
        "// Crafts {count_txt} × {}. Ingredients are the game's block/item ids (0 = empty).\nexport default class Block{}Crafting {{\n    static recipe = [{}];\n    static amount_output = {};\n    static shapeless = {};{}\n}}\n",
        s(&r["result"]),
        pascal(&s(&r["result"])),
        sh.list.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
        (if count == 0. { 1. } else { count }).clamp(1., 64.) as i64,
        shapeless,
        if shapeless { String::new() } else { format!("\n    static width = {};\n    static height = {};", sh.width, sh.height) }
    )
}

fn command(c: &Value) -> String {
    let reply: Vec<String> = s(&c["reply"]).split('\n').filter(|l| !l.trim().is_empty()).map(|l| format!("        minecraft.addMessageToChat({});", serde_json::to_string(l).unwrap())).collect();
    format!(
        "import Command from \"../Command.js\";\n\nexport default class Command{} extends Command {{\n\n    constructor() {{\n        super({}, \"\", {});\n        this.opOnly = false;\n    }}\n\n    execute(minecraft, args) {{\n{}\n        return true;\n    }}\n}}\n",
        pascal(&s(&c["name"])),
        str_lit(&c["name"]),
        str_lit(&c["description"]),
        if reply.is_empty() { "        // nothing to say".to_string() } else { reply.join("\n") }
    )
}

fn tab(p: &Value) -> String {
    let first = arr(&p["blocks"]).first().or(arr(&p["items"]).first());
    format!(
        "export default class Tab{} {{\n    static NAME = {};\n    static ICON_BLOCK_ID = '{}:{}';\n}}\n",
        pascal(&s(&p["modId"])),
        str_lit(&p["name"]),
        s(&p["modId"]),
        first.map(|f| game_id(&s(&f["key"]))).unwrap_or_default()
    )
}

/// Problems that would stop the mod from loading.
pub fn validate(p: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    if s(&p["name"]).trim().is_empty() {
        errs.push("Give the mod a name.".into());
    }
    if !valid_mod_id(&s(&p["modId"])) {
        errs.push("The mod ID must be 2–32 lowercase letters, numbers or _ and start with a letter.".into());
    }
    let mut keys = HashSet::new();
    let mut ids = HashSet::new();
    for x in arr(&p["blocks"]).iter().chain(arr(&p["items"])) {
        let key = s(&x["key"]);
        if !valid_key(&key) {
            errs.push(format!("“{}” needs an ID of lowercase letters, numbers or _.", if s(&x["name"]).is_empty() { key.clone() } else { s(&x["name"]) }));
        } else if ids.contains(&game_id(&key)) {
            errs.push(format!("Two things share the ID “{}”.", game_id(&key)));
        }
        ids.insert(game_id(&key));
        keys.insert(key);
    }
    let mut results = HashSet::new();
    for r in arr(&p["recipes"]) {
        let res = s(&r["result"]);
        if results.contains(&res) {
            errs.push(format!("Two recipes make “{res}”. Each block or item can have one recipe."));
        }
        results.insert(res.clone());
        if !keys.contains(&res) {
            errs.push("A recipe makes something that no longer exists.".into());
        } else if recipe_shape(r).list.is_empty() {
            errs.push(format!("The recipe for “{res}” is empty."));
        }
    }
    for c in arr(&p["commands"]) {
        let n = s(&c["name"]);
        let mut ch = n.chars();
        let ok = matches!(ch.next(), Some(f) if f.is_ascii_lowercase()) && n.len() <= 32 && ch.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            errs.push(format!("Command “/{n}” needs a lowercase name."));
        }
    }
    let mut paths = HashSet::new();
    for patch in arr(&p["patches"]) {
        match patch_path(&s(&patch["file"])) {
            None => errs.push("Each patch needs a valid game JavaScript file path.".into()),
            Some(path) if paths.contains(&path) => errs.push(format!("Two patches target “{path}”.")),
            Some(path) => {
                paths.insert(path);
            }
        }
        if s(&patch["source"]).trim().is_empty() {
            errs.push("Patch code cannot be empty.".into());
        }
    }
    errs
}

fn data_url_bytes(v: &Value) -> Option<Vec<u8>> {
    let src = s(v);
    let b64 = src.split_once(',').map(|(_, b)| b).unwrap_or(&src);
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()
}

/// path → file bytes, for `projects::save`.
pub fn generate(p: &Value) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert("ModData.js".to_string(), mod_data(p).into_bytes());
    files.insert("ModLoad.js".to_string(), mod_load(p).into_bytes());
    if !s(&p["icon"]).is_empty() {
        if let Some(b) = data_url_bytes(&p["icon"]) {
            files.insert("icon.png".into(), b);
        }
    }
    for b in arr(&p["blocks"]) {
        let key = s(&b["key"]);
        files.insert(format!("blocks/Block{}.js", pascal(&key)), block(p, b).into_bytes());
        if !s(&b["texture"]).is_empty() {
            if let Some(t) = data_url_bytes(&b["texture"]) {
                files.insert(format!("textures/{key}.png"), t);
            }
        }
    }
    for it in arr(&p["items"]) {
        let key = s(&it["key"]);
        files.insert(format!("items/Item{}.js", pascal(&key)), item(p, it).into_bytes());
        if !s(&it["texture"]).is_empty() {
            if let Some(t) = data_url_bytes(&it["texture"]) {
                files.insert(format!("textures/{key}.png"), t);
            }
        }
    }
    for r in arr(&p["recipes"]) {
        if !recipe_shape(r).list.is_empty() {
            files.insert(format!("crafting/Block{}Crafting.js", pascal(&s(&r["result"]))), crafting(r).into_bytes());
        }
    }
    for c in arr(&p["commands"]) {
        files.insert(format!("commands/Command{}.js", pascal(&s(&c["name"]))), command(c).into_bytes());
    }
    for patch in arr(&p["patches"]) {
        if let Some(path) = patch_path(&s(&patch["file"])) {
            if !s(&patch["source"]).trim().is_empty() {
                files.insert(format!("patches/{path}"), s(&patch["source"]).into_bytes());
            }
        }
    }
    if !arr(&p["blocks"]).is_empty() || !arr(&p["items"]).is_empty() {
        files.insert(format!("tabs/Tab{}.js", pascal(&s(&p["modId"]))), tab(p).into_bytes());
    }
    files
}

/// The generated source for one thing, for the "View code" panels.
pub fn preview(p: &Value, kind: &str, x: &Value) -> String {
    match kind {
        "block" => block(p, x),
        "item" => item(p, x),
        "recipe" => crafting(x),
        "command" => command(x),
        _ => mod_load(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Rust generator must write exactly what the web Mod Maker writes (testdata made with generate.js).
    #[test]
    fn matches_generate_js() {
        let p: Value = serde_json::from_str(include_str!("testdata/project.json")).unwrap();
        let expected: BTreeMap<String, String> = serde_json::from_str(include_str!("testdata/expected.json")).unwrap();
        let got = generate(&p);
        assert_eq!(got.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>());
        for (path, want) in &expected {
            let have = &got[path];
            match want.strip_prefix("b64:") {
                Some(b64) => assert_eq!(*have, base64::engine::general_purpose::STANDARD.decode(b64).unwrap(), "{path}"),
                None => assert_eq!(String::from_utf8_lossy(have), *want, "{path}"),
            }
        }
    }

    #[test]
    fn ids_follow_the_game() {
        assert_eq!(game_id("ruby_block_2"), "ruby_block2");
        assert_eq!(to_key("  Ruby  Block!! "), "ruby_block");
        assert_eq!(patch_path("./src/js/a/B.js").as_deref(), Some("a/B.js"));
        assert_eq!(patch_path("../x.js"), None);
    }
}
