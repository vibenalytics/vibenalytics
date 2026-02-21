use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use chrono::Utc;

/// Returns the data directory for config, metrics, and logs.
/// Precedence: $XDG_DATA_HOME/vibenalytics → ~/.config/vibenalytics → %APPDATA%\vibenalytics → binary dir
pub fn data_dir() -> PathBuf {
    let dir = if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("vibenalytics")
    } else if let Ok(home) = env::var("HOME") {
        PathBuf::from(home).join(".config").join("vibenalytics")
    } else if let Ok(appdata) = env::var("APPDATA") {
        PathBuf::from(appdata).join("vibenalytics")
    } else {
        let exe_raw = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let exe = fs::canonicalize(&exe_raw).unwrap_or(exe_raw);
        exe.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn metrics_path(dir: &Path) -> PathBuf {
    dir.join("metrics.jsonl")
}

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join(".sync-config.json")
}

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("sync.log")
}

pub fn cursors_path(dir: &Path) -> PathBuf {
    dir.join("transcript-cursors.json")
}

pub fn projects_path(dir: &Path) -> PathBuf {
    dir.join("projects.json")
}

pub fn sync_log(dir: &Path, msg: &str) {
    let path = log_path(dir);
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// Returns the ~/.claude directory path.
pub fn claude_dir() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        PathBuf::from(home).join(".claude")
    } else {
        PathBuf::from(".claude")
    }
}
