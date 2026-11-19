use serde::Serialize;
use std::{fs, path::Path};

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())
}
