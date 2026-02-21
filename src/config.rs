use std::fs;
use std::io;
use std::path::Path;
use serde_json::Value;
use crate::paths::config_path;

pub const DEFAULT_API_BASE: &str = match option_env!("API_BASE") {
    Some(url) => url,
    None => "http://localhost:3001/api",
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
