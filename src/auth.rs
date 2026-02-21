use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::SystemTime;
use serde_json::json;
use crate::config::{write_config, DEFAULT_API_BASE};
use crate::paths::{sync_log, config_path};

pub fn generate_nonce() -> String {
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 16];
        if io::Read::read_exact(&mut f, &mut buf).is_ok() {
            return buf.iter().map(|b| format!("{:02x}", b)).collect();
        }
    }
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{:016x}{:08x}", ts, pid)
}

fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        match b {
            b'%' => {
                let hi = chars.next().unwrap_or(b'0');
                let lo = chars.next().unwrap_or(b'0');
                let hex = [hi, lo];
                if let Ok(s) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        result.push(val as char);
                        continue;
                    }
                }
                result.push('%');
                result.push(hi as char);
                result.push(lo as char);
            }
            b'+' => result.push(' '),
            _ => result.push(b as char),
        }
    }
    result
}

// ---- Non-blocking login for TUI ----

pub struct LoginListener {
    pub listener: TcpListener,
    pub nonce: String,
}

/// Start the login flow: bind port, open browser, return listener for polling.
pub fn start_login() -> Result<LoginListener, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind port: {e}"))?;
    let port = listener.local_addr().unwrap().port();
    let nonce = generate_nonce();

    let auth_url = format!("{DEFAULT_API_BASE}/auth/github?port={port}&state={nonce}");
    let _ = open::that(&auth_url);

    listener.set_nonblocking(true).ok();
    Ok(LoginListener { listener, nonce })
}

/// Poll the login listener. Returns Ok(Some((key, name))) on success,
/// Ok(None) if still waiting, Err on failure.
pub fn poll_login(login: &LoginListener) -> Result<Option<(String, String)>, String> {
    let stream = match login.listener.accept() {
        Ok((s, _)) => s,
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(e) => return Err(format!("Connection error: {e}")),
    };

    stream.set_nonblocking(false).ok();
    let mut reader = io::BufReader::new(&stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).map_err(|e| format!("Read error: {e}"))?;

    let path = request_line.split_whitespace().nth(1)
        .ok_or_else(|| "Invalid request".to_string())?;
    let query = path.splitn(2, '?').nth(1).unwrap_or("");
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|p| {
            let mut kv = p.splitn(2, '=');
            Some((kv.next()?.to_string(), urldecode(kv.next().unwrap_or(""))))
        })
        .collect();

    let callback_state = params.get("state").cloned().unwrap_or_default();
    if callback_state != login.nonce {
        let err_html = r#"<!DOCTYPE html><html><body><p>Authorization failed: invalid state.</p></body></html>"#;
        let err_resp = format!("HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err_html.len(), err_html);
        let _ = (&stream).write_all(err_resp.as_bytes());
        let _ = (&stream).flush();
        return Err("State mismatch (possible CSRF)".to_string());
    }

    let api_key = params.get("key").cloned().unwrap_or_default();
    let user_name = params.get("name").cloned().unwrap_or_else(|| "user".to_string());

    let html = r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Vibenalytics</title>
<style>body{font-family:system-ui;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#1a1a2e;color:#e0e0e0}
.card{text-align:center;padding:2rem;border-radius:12px;background:#252540;border:1px solid #333}.ok{color:#c97856;font-size:1.5rem;margin-bottom:0.5rem}</style>
</head><body><div class="card"><div class="ok">CLI Authorized!</div><p>You can close this tab and return to your terminal.</p></div></body></html>"#;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(), html
    );
    let _ = (&stream).write_all(response.as_bytes());
    let _ = (&stream).flush();
    drop(stream);

    if api_key.is_empty() {
        return Err("No API key received".to_string());
    }

    Ok(Some((api_key, user_name)))
}

/// Save login credentials to config.
pub fn save_login(dir: &Path, api_key: &str, display_name: &str) -> Result<(), String> {
    let cfg = json!({
        "apiKey": api_key,
        "displayName": display_name,
    });
    write_config(dir, &cfg).map_err(|e| format!("Failed to write config: {e}"))?;
    sync_log(dir, &format!("Browser login: {display_name}"));
    Ok(())
}

// ---- CLI commands (for non-TUI use) ----

pub fn cmd_login(dir: &Path) -> i32 {
    let login = match start_login() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    let port = login.listener.local_addr().unwrap().port();
    let auth_url = format!("{DEFAULT_API_BASE}/auth/github?port={port}&state={}", login.nonce);
    eprintln!("Opening browser for authentication...\n");
    eprintln!("If the browser didn't open, visit this URL:");
    eprintln!("  {auth_url}\n");
    eprintln!("Waiting for authorization (press Ctrl+C to cancel)...");

    let timeout = std::time::Duration::from_secs(300);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            eprintln!("\nTimed out waiting for authorization.");
            return 1;
        }
        match poll_login(&login) {
            Ok(Some((key, name))) => {
                if let Err(e) = save_login(dir, &key, &name) {
                    eprintln!("{e}");
                    return 1;
                }
                eprintln!("\nLogged in as {name}");
                return 0;
            }
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                eprintln!("\n{e}");
                return 1;
            }
        }
    }
}

pub fn cmd_logout(dir: &Path) -> i32 {
    let path = config_path(dir);
    if !path.exists() {
        eprintln!("Not logged in.");
        return 0;
    }
    if let Err(e) = fs::remove_file(&path) {
        eprintln!("Failed to remove config: {e}");
        return 1;
    }
    eprintln!("Logged out. Credentials removed.");
    sync_log(dir, "Logged out");
    0
}
