use std::fs;
use std::io;
use std::path::Path;
use serde_json::Value;
use crate::paths::config_path;

pub const DEFAULT_API_BASE: &str = match option_env!("API_BASE") {
    Some(url) => url,
    None => "http://localhost:3001/api",
};

pub const APP_NAME: &str = match option_env!("APP_NAME") {
    Some(name) => name,
    None => "vibenalytics",
};

pub const DEFAULT_FRONTEND_BASE: &str = match option_env!("FRONTEND_BASE") {
    Some(url) => url,
    None => "http://localhost:3000",
};


pub fn read_config(dir: &Path) -> Option<Value> {
    let data = fs::read_to_string(config_path(dir)).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn write_config(dir: &Path, cfg: &Value) -> io::Result<()> {
    let path = config_path(dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(cfg)?;
    fs::write(path, data)
}

pub fn config_get(dir: &Path, key: &str) -> Option<String> {
    read_config(dir)?
        .get(key)?
        .as_str()
        .map(|s| s.to_string())
}

pub fn config_get_bool(dir: &Path, key: &str) -> bool {
    config_get_bool_default(dir, key, false)
}

pub fn config_get_bool_default(dir: &Path, key: &str, default: bool) -> bool {
    read_config(dir)
        .and_then(|cfg| cfg.get(key)?.as_bool())
        .unwrap_or(default)
}

pub fn config_set_bool(dir: &Path, key: &str, value: bool) -> io::Result<()> {
    let mut cfg = read_config(dir).unwrap_or(serde_json::json!({}));
    cfg[key] = serde_json::Value::Bool(value);
    write_config(dir, &cfg)
}

pub fn config_set(dir: &Path, key: &str, value: &str) -> io::Result<()> {
    let mut cfg = read_config(dir).unwrap_or(serde_json::json!({}));
    cfg[key] = serde_json::Value::String(value.to_string());
    write_config(dir, &cfg)
}
