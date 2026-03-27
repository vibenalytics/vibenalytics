use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;
use chrono::Utc;

const REPO: &str = "vibenalytics/vibenalytics";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_KEPT_VERSIONS: usize = 2;

// ---- Platform detection ----

fn platform_artifact() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("vibenalytics-darwin-arm64"),
        ("macos", "x86_64") => Some("vibenalytics-darwin-x64"),
        ("linux", "x86_64") => Some("vibenalytics-linux-x64"),
        ("linux", "aarch64") => Some("vibenalytics-linux-arm64"),
        _ => None,
    }
}

// ---- Version directory layout ----
//
//   ~/.local/share/vibenalytics/versions/
//     0.10.0      (binary)
//     0.11.0      (binary, active)
//
//   ~/.local/bin/vibenalytics -> ~/.local/share/vibenalytics/versions/0.11.0  (symlink)

fn versions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/vibenalytics/versions")
}

fn default_link_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/bin/vibenalytics")
}

// ---- GitHub API ----

fn fetch_latest_tag() -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = ureq::get(&url)
        .set("User-Agent", "vibenalytics")
        .call()
        .map_err(|e| format!("Failed to check for updates: {e}"))?;
    let body: serde_json::Value = resp.into_json().map_err(|e| format!("Invalid response: {e}"))?;
    body.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No tag_name in release".to_string())
}

// ---- Download ----

fn download_and_extract(tag: &str, artifact: &str, dest: &Path) -> Result<(), String> {
    let url = format!(
        "https://github.com/{REPO}/releases/download/{tag}/{artifact}.tar.gz"
    );

    let resp = ureq::get(&url)
        .set("User-Agent", "vibenalytics")
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;

    let mut compressed = Vec::new();
    resp.into_reader()
        .read_to_end(&mut compressed)
        .map_err(|e| format!("Read failed: {e}"))?;

    let tmp_dir = dest.parent().unwrap_or(Path::new("/tmp"));
    let tmp_tar = tmp_dir.join(".vibenalytics-update.tar.gz");
    fs::write(&tmp_tar, &compressed).map_err(|e| format!("Write temp failed: {e}"))?;

    let output = Command::new("tar")
        .args(["xzf", &tmp_tar.to_string_lossy(), "-C", &tmp_dir.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("tar extract failed: {e}"))?;

    let _ = fs::remove_file(&tmp_tar);

    if !output.status.success() {
        return Err(format!(
            "tar extract failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let extracted = tmp_dir.join(artifact);
    if !extracted.exists() {
        return Err("Extracted binary not found".to_string());
    }

    fs::rename(&extracted, dest).map_err(|e| format!("Move failed: {e}"))?;

    Ok(())
}

// ---- Atomic symlink swap ----
// Creates a temp symlink in the same directory, then rename() over the target.
// rename() is atomic per POSIX — no gap where the path doesn't exist.

fn atomic_symlink(target: &Path, link_path: &Path) -> Result<(), String> {
    // Check if already pointing to the right target
    if let Ok(existing) = fs::read_link(link_path) {
        if existing == target {
            return Ok(());
        }
    }

    let link_dir = link_path.parent().ok_or("No parent directory for link")?;
    let _ = fs::create_dir_all(link_dir);

    // Temp symlink: same directory as final link (same filesystem for rename)
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_name = format!(
        ".vibenalytics.tmp.{}.{}",
        std::process::id(),
        now
    );
    let temp_path = link_dir.join(&temp_name);

    // Clean up any stale temp file from a previous crash
    let _ = fs::remove_file(&temp_path);

    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &temp_path)
        .map_err(|e| format!("Failed to create temp symlink: {e}"))?;

    #[cfg(not(unix))]
    return Err("Symlink updates not supported on this platform".to_string());

    // Atomic swap: rename temp symlink over the existing path (file or symlink)
    if let Err(e) = fs::rename(&temp_path, link_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Atomic symlink swap failed: {e}"));
    }

    Ok(())
}

// ---- Install a version ----
// Downloads to versions dir, sets permissions, atomically swaps symlink.

fn install_version(tag: &str, link_path: &Path, verbose: bool) -> Result<(), String> {
    let artifact = platform_artifact()
        .ok_or_else(|| format!("Unsupported platform: {} {}", std::env::consts::OS, std::env::consts::ARCH))?;

    let version = tag.trim_start_matches('v');
    let ver_dir = versions_dir();
    let _ = fs::create_dir_all(&ver_dir);
    let version_bin = ver_dir.join(version);

    // Skip if this version is already downloaded
    if version_bin.exists() {
        if verbose {
            eprintln!("Version {tag} already downloaded.");
        }
    } else {
        if verbose {
            eprintln!("Downloading {artifact} {tag}...");
        }
        download_and_extract(tag, artifact, &version_bin)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&version_bin, fs::Permissions::from_mode(0o755));
        }
    }

    // Migrate: if link_path is a regular file (old install), remove it first
    if link_path.exists() && !link_path.is_symlink() {
        // Move the old binary into versions dir as the current version for rollback
        let old_version_path = ver_dir.join(CURRENT_VERSION);
        if !old_version_path.exists() {
            let _ = fs::rename(link_path, &old_version_path);
        } else {
            let _ = fs::remove_file(link_path);
        }
    }

    // Atomic symlink swap
    atomic_symlink(&version_bin, link_path)?;

    // Clean up old versions
    cleanup_old_versions(&ver_dir, version);

    Ok(())
}

// ---- Cleanup ----
// Keep MAX_KEPT_VERSIONS most recent versions. Never delete the active version.

fn cleanup_old_versions(ver_dir: &Path, active_version: &str) {
    let entries = match fs::read_dir(ver_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut versions: Vec<(String, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip temp files and non-version entries
        if name.starts_with('.') {
            continue;
        }
        let mtime = entry.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        versions.push((name, mtime));
    }

    // Sort newest first
    versions.sort_by(|a, b| b.1.cmp(&a.1));

    for (i, (name, _)) in versions.iter().enumerate() {
        // Keep active version and the N most recent
        if name == active_version || i < MAX_KEPT_VERSIONS {
            continue;
        }
        let _ = fs::remove_file(ver_dir.join(name));
    }
}

// ---- Resolve the link path (where the symlink lives) ----
// Uses current_exe if it's a symlink (production install),
// otherwise falls back to the default path.

fn resolve_link_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        // If we're running via a symlink, use that symlink's path
        if exe.is_symlink() || fs::read_link(&exe).is_ok() {
            return exe;
        }
        // If the raw (non-canonicalized) exe is a symlink
        // (current_exe() may or may not resolve symlinks depending on OS)
        let raw = std::env::current_exe().unwrap_or_default();
        if raw.is_symlink() || fs::read_link(&raw).is_ok() {
            return raw;
        }
    }
    default_link_path()
}

// ---- Public: manual update command ----

pub fn cmd_update() -> i32 {
    eprintln!("Current version: v{CURRENT_VERSION}");
    eprintln!("Checking for updates...");

    let tag = match fetch_latest_tag() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    let remote_version = tag.trim_start_matches('v');
    if remote_version == CURRENT_VERSION {
        eprintln!("Already up to date (v{CURRENT_VERSION}).");
        return 0;
    }

    eprintln!("New version available: {tag}");

    let link_path = resolve_link_path();

    match install_version(&tag, &link_path, true) {
        Ok(()) => {
            eprintln!("Updated to {tag}!");
            0
        }
        Err(e) => {
            eprintln!("Update failed: {e}");
            1
        }
    }
}

// ---- Public: background auto-update ----

/// Runs as a detached process (`vibenalytics _update-check`).
/// Checks GitHub, downloads, and atomically swaps the symlink.
pub fn cmd_background_update_check(dir: &Path) -> i32 {
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let _ = crate::config::config_set(dir, "lastUpdateCheck", &now);

    // Skip dev builds (symlink pointing into target/release/)
    if is_dev_build() {
        return 0;
    }

    let tag = match fetch_latest_tag() {
        Ok(t) => t,
        Err(_) => return 1,
    };

    let remote_version = tag.trim_start_matches('v');
    if remote_version == CURRENT_VERSION {
        crate::paths::sync_log(dir, &format!("[update] Up to date (v{CURRENT_VERSION})"));
        return 0;
    }

    crate::paths::sync_log(dir, &format!("[update] New version available: {tag} (current: v{CURRENT_VERSION})"));
    let link_path = resolve_link_path();

    match install_version(&tag, &link_path, false) {
        Ok(()) => {
            crate::paths::sync_log(dir, &format!("[update] Updated to {tag}"));
            0
        }
        Err(e) => {
            crate::paths::sync_log(dir, &format!("[update] Failed: {e}"));
            1
        }
    }
}

/// Called on SessionStart. Spawns background update check if stale (> 24h).
pub fn auto_update(dir: &Path) {
    if is_dev_build() {
        return;
    }

    // Skip if checked recently
    let last = crate::config::config_get(dir, "lastUpdateCheck").unwrap_or_default();
    if !last.is_empty() {
        let is_fresh = chrono::DateTime::parse_from_rfc3339(&last)
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&last, "%Y-%m-%dT%H:%M:%SZ")
                .map(|dt| dt.and_utc().fixed_offset()))
            .map(|dt| Utc::now().signed_duration_since(dt.with_timezone(&Utc)) < chrono::Duration::hours(24))
            .unwrap_or(false);
        if is_fresh {
            return;
        }
    }

    // Fire-and-forget: spawn detached process
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    crate::paths::sync_log(dir, "[update] Spawning background update check");
    let _ = Command::new(exe)
        .args(["_update-check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

// ---- Helpers ----

/// Dev builds are symlinks pointing into a cargo target directory.
fn is_dev_build() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    match fs::read_link(&exe) {
        Ok(target) => {
            let target_str = target.to_string_lossy();
            target_str.contains("/target/release/") || target_str.contains("/target/debug/")
        }
        Err(_) => false,
    }
}
