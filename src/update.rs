use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};

const REPO: &str = "vibenalytics/vibenalytics";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn platform_artifact() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("vibenalytics-darwin-arm64"),
        ("macos", "x86_64") => Some("vibenalytics-darwin-x64"),
        ("linux", "x86_64") => Some("vibenalytics-linux-x64"),
        ("linux", "aarch64") => Some("vibenalytics-linux-arm64"),
        _ => None,
    }
}

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

fn download_and_extract(tag: &str, artifact: &str, dest: &std::path::Path) -> Result<(), String> {
    let url = format!(
        "https://github.com/{REPO}/releases/download/{tag}/{artifact}.tar.gz"
    );
    eprintln!("Downloading {artifact} {tag}...");

    let resp = ureq::get(&url)
        .set("User-Agent", "vibenalytics")
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;

    let mut compressed = Vec::new();
    resp.into_reader()
        .read_to_end(&mut compressed)
        .map_err(|e| format!("Read failed: {e}"))?;

    // Write tar.gz to temp file, extract with system tar
    let tmp_dir = dest.parent().unwrap_or(std::path::Path::new("/tmp"));
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

    // The extracted file is named after the artifact
    let extracted = tmp_dir.join(artifact);
    if !extracted.exists() {
        return Err("Extracted binary not found".to_string());
    }

    fs::rename(&extracted, dest).map_err(|e| format!("Move failed: {e}"))?;

    Ok(())
}

pub fn cmd_update() -> i32 {
    let artifact = match platform_artifact() {
        Some(a) => a,
        None => {
            eprintln!(
                "Unsupported platform: {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            return 1;
        }
    };

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

    let current_exe = match std::env::current_exe() {
        Ok(p) => fs::canonicalize(&p).unwrap_or(p),
        Err(e) => {
            eprintln!("Cannot determine binary path: {e}");
            return 1;
        }
    };

    // Don't update symlinks (dev builds)
    if let Ok(orig) = std::fs::read_link(std::env::current_exe().unwrap_or_default()) {
        eprintln!(
            "Current binary is a symlink to {:?}.\nUse the install script instead:\n  curl -fsSL https://vibenalytics.dev/install.sh | bash",
            orig
        );
        return 1;
    }

    // Backup current binary
    let backup = current_exe.with_extension("old");
    if let Err(e) = fs::rename(&current_exe, &backup) {
        eprintln!("Failed to backup current binary: {e}");
        return 1;
    }

    match download_and_extract(&tag, artifact, &current_exe) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755));
            }
            let _ = fs::remove_file(&backup);
            eprintln!("Updated to {tag}!");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            let _ = fs::rename(&backup, &current_exe);
            1
        }
    }
}
