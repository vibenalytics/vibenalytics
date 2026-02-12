/// sync-tool v2 — Claudnalytics metrics logger, aggregator and sync client
///
/// Single Rust binary. Zero runtime dependencies.
///
/// Usage:
///   sync-tool log                                  Read hook JSON from stdin, strip content, append to metrics.jsonl
///   sync-tool sync                                 Aggregate + POST + flush
///   sync-tool login <email> <password>             Login to get API key
///   sync-tool login --api-key <key>                Set API key directly
///   sync-tool status                               Show registration
///   sync-tool aggregate <file>                     Dump aggregated JSON to stdout
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::Utc;
use serde_json::{json, Map, Value};

// ---- Path helpers ----

fn resolve_project_dir() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    // If we're in target/release or target/debug, go up
    if dir.ends_with("release") || dir.ends_with("debug") {
        dir = dir.parent().unwrap().to_path_buf(); // target/
        dir = dir.parent().unwrap().to_path_buf(); // native/
    }
    // dir = native/, go up to claudnalytics/, then to project root
    let project = dir
        .parent() // claudnalytics/
        .and_then(|p| p.parent()) // project root
        .unwrap_or(Path::new("."));
    project.to_path_buf()
}

fn metrics_path(project_dir: &Path) -> PathBuf {
    project_dir.join("metrics.jsonl")
}

fn config_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("claudnalytics")
        .join("native")
        .join(".sync-config.json")
}

fn log_path(project_dir: &Path) -> PathBuf {
    project_dir.join("claudnalytics").join("native").join("sync.log")
}

// ---- Logging ----

fn sync_log(project_dir: &Path, msg: &str) {
    let path = log_path(project_dir);
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

// ---- Config ----

fn read_config(project_dir: &Path) -> Option<Value> {
    let data = fs::read_to_string(config_path(project_dir)).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_config(project_dir: &Path, cfg: &Value) -> io::Result<()> {
    let path = config_path(project_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(cfg)?;
    fs::write(path, data)
}

fn config_get(project_dir: &Path, key: &str) -> Option<String> {
    read_config(project_dir)?
        .get(key)?
        .as_str()
        .map(|s| s.to_string())
}

// ---- HTTP ----

fn http_post(url: &str, body: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    let mut req = ureq::post(url).set("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.set("X-API-Key", key);
    }
    let resp = req.send_string(body).map_err(|e| format!("{e}"))?;
    let status = resp.status();
    let body = resp.into_string().unwrap_or_default();
    Ok((status, body))
}

// ---- Log command ----

fn strip_field_bytes(obj: &mut Map<String, Value>, key: &str) {
    if let Some(val) = obj.remove(key) {
        let bytes = match &val {
            Value::String(s) => s.len(),
            other => serde_json::to_string(other).map(|s| s.len()).unwrap_or(0),
        };
        let type_name = match &val {
            Value::String(_) => "string",
            _ => "other",
        };
        obj.insert(
            key.to_string(),
            json!({"_bytes": bytes, "_type": type_name}),
        );
    }
}

fn strip_command(obj: &mut Map<String, Value>) {
    if let Some(Value::String(cmd)) = obj.remove("command") {
        let bytes = cmd.len();
        let first_token = cmd.split_whitespace().next().unwrap_or("");
        let bin = first_token.rsplit('/').next().unwrap_or(first_token);
        let preview: String = cmd.chars().take(80).collect();
        obj.insert(
            "command".to_string(),
            json!({"_bytes": bytes, "_bin": bin, "_preview": preview}),
        );
    }
}

fn strip_long_string(obj: &mut Map<String, Value>, key: &str, max_len: usize, preview_len: usize) {
    let should_strip = obj
        .get(key)
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.len() > max_len);
    if !should_strip {
        return;
    }
    if let Some(Value::String(s)) = obj.remove(key) {
        let bytes = s.len();
        let mut replacement = json!({"_bytes": bytes});
        if preview_len > 0 {
            let preview: String = s.chars().take(preview_len).collect();
            replacement["_preview"] = Value::String(preview);
        }
        obj.insert(key.to_string(), replacement);
    }
}

fn cmd_log(project_dir: &Path) -> i32 {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() || input.is_empty() {
        return 0;
    }

    let mut evt: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => {
            let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let line = json!({"logged_at": ts, "parse_error": true});
            let path = metrics_path(project_dir);
            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{}", line);
            }
            return 0;
        }
    };

    let obj = match evt.as_object_mut() {
        Some(o) => o,
        None => return 0,
    };

    obj.insert("_input_bytes".to_string(), json!(input.len()));

    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    obj.insert("logged_at".to_string(), json!(ts));

    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string();
    obj.insert("hostname".to_string(), json!(hostname));

    // Strip tool_input content fields
    if let Some(Value::Object(ti)) = obj.get_mut("tool_input") {
        strip_field_bytes(ti, "content");
        strip_field_bytes(ti, "old_string");
        strip_field_bytes(ti, "new_string");
        strip_field_bytes(ti, "new_source");
        strip_command(ti);
        strip_long_string(ti, "prompt", 200, 0);
        strip_long_string(ti, "description", 300, 100);
    }

    // Strip tool_response → tool_response_bytes
    if let Some(resp) = obj.remove("tool_response") {
        let bytes = serde_json::to_string(&resp).map(|s| s.len()).unwrap_or(0);
        obj.insert("tool_response_bytes".to_string(), json!(bytes));
    }

    // Strip transcript_path → project name
    if let Some(Value::String(tp)) = obj.remove("transcript_path") {
        let parts: Vec<&str> = tp.split('/').filter(|s| !s.is_empty()).collect();
        let project = if parts.len() >= 2 {
            parts[parts.len() - 2]
        } else {
            "unknown"
        };
        obj.insert("project".to_string(), json!(project));
    }

    // Strip user prompt content
    let is_prompt_submit = obj
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == "UserPromptSubmit");
    if is_prompt_submit {
        if let Some(Value::String(prompt)) = obj.remove("prompt") {
            obj.insert("prompt_bytes".to_string(), json!(prompt.len()));
        }
    }

    let path = metrics_path(project_dir);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", serde_json::to_string(&evt).unwrap_or_default());
    }

    0
}

// ---- Aggregation ----

struct ToolLatency {
    tool: String,
    total_ms: u64,
    count: u32,
    min_ms: u64,
    max_ms: u64,
}

struct PermissionStat {
    tool: String,
    domain: String,
    count: u32,
}

struct Session {
    session_id: String,
    project: String,
    started_at: String,
    ended_at: String,
    permission_mode: String,
    events: HashMap<String, u32>,
    tools: HashMap<String, u32>,
    prompt_count: u32,
    total_input_bytes: u64,
    total_response_bytes: u64,
    tool_latencies: Vec<ToolLatency>,
    permission_requests: Vec<PermissionStat>,
    tool_response_sizes: HashMap<String, (u64, u32)>,
    parallel_tool_batches: u32,
}

impl Session {
    fn new(id: &str) -> Self {
        Session {
            session_id: id.to_string(),
            project: "unknown".to_string(),
            started_at: String::new(),
            ended_at: String::new(),
            permission_mode: String::new(),
            events: HashMap::new(),
            tools: HashMap::new(),
            prompt_count: 0,
            total_input_bytes: 0,
            total_response_bytes: 0,
            tool_latencies: Vec::new(),
            permission_requests: Vec::new(),
            tool_response_sizes: HashMap::new(),
            parallel_tool_batches: 0,
        }
    }
}

fn aggregate_file(filepath: &Path) -> Vec<Session> {
    let content = match fs::read_to_string(filepath) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut sessions: Vec<Session> = Vec::new();
    let mut session_map: HashMap<String, usize> = HashMap::new();
    let mut pre_tool_times: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut pending_tools: HashMap<String, u32> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let evt: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let sid = evt
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let idx = if let Some(&i) = session_map.get(sid) {
            i
        } else {
            let i = sessions.len();
            sessions.push(Session::new(sid));
            session_map.insert(sid.to_string(), i);
            i
        };
        let s = &mut sessions[idx];

        let ts_str = evt.get("logged_at").and_then(|v| v.as_str()).unwrap_or("");
        if !ts_str.is_empty() {
            if s.started_at.is_empty() || ts_str < s.started_at.as_str() {
                s.started_at = ts_str.to_string();
            }
            if s.ended_at.is_empty() || ts_str > s.ended_at.as_str() {
                s.ended_at = ts_str.to_string();
            }
        }

        if let Some(proj) = evt.get("project").and_then(|v| v.as_str()) {
            if proj != "unknown" {
                s.project = proj.to_string();
            }
        } else if s.project == "unknown" {
            if let Some(cwd) = evt.get("cwd").and_then(|v| v.as_str()) {
                if let Some(last) = cwd.rsplit('/').next() {
                    if !last.is_empty() {
                        s.project = last.to_string();
                    }
                }
            }
        }

        let event = evt
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !event.is_empty() {
            *s.events.entry(event.to_string()).or_insert(0) += 1;
        }

        let tool_name = evt
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tool_use_id = evt
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if event == "PreToolUse" && !tool_use_id.is_empty() && !ts_str.is_empty() {
            pre_tool_times.insert(
                (sid.to_string(), tool_use_id.to_string()),
                (tool_name.to_string(), ts_str.to_string()),
            );
            let pending = pending_tools.entry(sid.to_string()).or_insert(0);
            *pending += 1;
            if *pending > 1 {
                s.parallel_tool_batches += 1;
            }
        }

        if (event == "PostToolUse" || event == "PostToolUseFailure") && !tool_name.is_empty() {
            if event == "PostToolUse" {
                *s.tools.entry(tool_name.to_string()).or_insert(0) += 1;
            } else {
                *s.tools
                    .entry(format!("{tool_name}_FAILED"))
                    .or_insert(0) += 1;
            }

            if !tool_use_id.is_empty() && !ts_str.is_empty() {
                let key = (sid.to_string(), tool_use_id.to_string());
                if let Some((pre_tool, pre_ts)) = pre_tool_times.remove(&key) {
                    if let (Some(start_epoch), Some(end_epoch)) = (
                        parse_iso_timestamp(&pre_ts),
                        parse_iso_timestamp(ts_str),
                    ) {
                        let latency_ms = ((end_epoch - start_epoch) * 1000) as u64;
                        if let Some(tl) = s.tool_latencies.iter_mut().find(|t| t.tool == pre_tool) {
                            tl.total_ms += latency_ms;
                            tl.count += 1;
                            tl.min_ms = tl.min_ms.min(latency_ms);
                            tl.max_ms = tl.max_ms.max(latency_ms);
                        } else {
                            s.tool_latencies.push(ToolLatency {
                                tool: pre_tool,
                                total_ms: latency_ms,
                                count: 1,
                                min_ms: latency_ms,
                                max_ms: latency_ms,
                            });
                        }
                    }
                }
                if let Some(pending) = pending_tools.get_mut(sid) {
                    *pending = pending.saturating_sub(1);
                }
            }

            if let Some(rb) = evt.get("tool_response_bytes").and_then(|v| v.as_u64()) {
                let entry = s
                    .tool_response_sizes
                    .entry(tool_name.to_string())
                    .or_insert((0, 0));
                entry.0 += rb;
                entry.1 += 1;
            }
        }

        if event == "PermissionRequest" {
            let perm_tool = tool_name.to_string();
            let domain = evt
                .get("permission_suggestions")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|s| s.get("rules"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|r| r.get("ruleContent"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(ps) = s
                .permission_requests
                .iter_mut()
                .find(|p| p.tool == perm_tool && p.domain == domain)
            {
                ps.count += 1;
            } else {
                s.permission_requests.push(PermissionStat {
                    tool: perm_tool,
                    domain,
                    count: 1,
                });
            }
        }

        if event == "UserPromptSubmit" {
            s.prompt_count += 1;
        }

        if let Some(ib) = evt.get("_input_bytes").and_then(|v| v.as_u64()) {
            s.total_input_bytes += ib;
        }
        if let Some(rb) = evt.get("tool_response_bytes").and_then(|v| v.as_u64()) {
            s.total_response_bytes += rb;
        }

        if let Some(pm) = evt.get("permission_mode").and_then(|v| v.as_str()) {
            s.permission_mode = pm.to_string();
        }
    }

    sessions
}

fn parse_iso_timestamp(ts: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

fn build_payload(sessions: &[Session]) -> Value {
    let arr: Vec<Value> = sessions
        .iter()
        .map(|s| {
            let mut obj = json!({
                "session_id": s.session_id,
                "project": s.project,
                "started_at": s.started_at,
                "ended_at": s.ended_at,
                "events": s.events,
                "tools": s.tools,
                "prompt_count": s.prompt_count,
                "total_input_bytes": s.total_input_bytes,
                "total_response_bytes": s.total_response_bytes,
            });
            if !s.permission_mode.is_empty() {
                obj["permission_mode"] = json!(s.permission_mode);
            }
            if let (Some(start), Some(end)) = (
                parse_iso_timestamp(&s.started_at),
                parse_iso_timestamp(&s.ended_at),
            ) {
                obj["duration_seconds"] = json!(end - start);
            }
            if !s.tool_latencies.is_empty() {
                let latencies: Vec<Value> = s.tool_latencies.iter().map(|tl| {
                    json!({
                        "tool": tl.tool,
                        "avg_ms": if tl.count > 0 { tl.total_ms / tl.count as u64 } else { 0 },
                        "min_ms": tl.min_ms,
                        "max_ms": tl.max_ms,
                        "count": tl.count,
                    })
                }).collect();
                obj["tool_latencies"] = json!(latencies);
            }
            if !s.permission_requests.is_empty() {
                let perms: Vec<Value> = s.permission_requests.iter().map(|p| {
                    json!({ "tool": p.tool, "domain": p.domain, "count": p.count })
                }).collect();
                obj["permission_requests"] = json!(perms);
            }
            if !s.tool_response_sizes.is_empty() {
                let sizes: Vec<Value> = s.tool_response_sizes.iter().map(|(tool, (total, count))| {
                    json!({
                        "tool": tool,
                        "total_bytes": total,
                        "avg_bytes": if *count > 0 { total / *count as u64 } else { 0 },
                        "count": count,
                    })
                }).collect();
                obj["tool_response_sizes"] = json!(sizes);
            }
            if s.parallel_tool_batches > 0 {
                obj["parallel_tool_batches"] = json!(s.parallel_tool_batches);
            }
            obj
        })
        .collect();
    json!({ "sessions": arr })
}

// ---- Commands ----

fn cmd_aggregate(filepath: &str) -> i32 {
    let sessions = aggregate_file(Path::new(filepath));
    if sessions.is_empty() {
        eprintln!("No events found in {filepath}");
        return 1;
    }
    let payload = build_payload(&sessions);
    println!("{}", serde_json::to_string(&payload).unwrap_or_default());
    0
}

fn cmd_login_with_credentials(project_dir: &Path, email: &str, password: &str) -> i32 {
    let api = config_get(project_dir, "apiBase")
        .unwrap_or_else(|| "http://localhost:3001/api".to_string());

    println!("Logging in as \"{email}\"...");

    // Step 1: Login to get JWT
    let login_body = json!({"email": email, "password": password});
    let url = format!("{api}/auth/login");
    let (status, resp) = match http_post(&url, &login_body.to_string(), None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Login failed: {e}");
            return 1;
        }
    };

    if status != 200 {
        eprintln!("Login failed (HTTP {status}): {resp}");
        return 1;
    }

    let data: Value = match serde_json::from_str(&resp) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Invalid response");
            return 1;
        }
    };

    let token = data
        .pointer("/data/token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let user_name = data
        .pointer("/data/user/name")
        .and_then(|v| v.as_str())
        .unwrap_or("user");

    if token.is_empty() {
        eprintln!("No token in response");
        return 1;
    }

    // Step 2: Generate API key
    let key_url = format!("{api}/auth/api-key");
    let key_resp = ureq::post(&key_url)
        .set("Content-Type", "application/json")
        .set("Authorization", &format!("Bearer {token}"))
        .send_string("{}");

    let api_key = match key_resp {
        Ok(r) => {
            let body = r.into_string().unwrap_or_default();
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            v.pointer("/data/apiKey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        Err(e) => {
            eprintln!("Failed to get API key: {e}");
            return 1;
        }
    };

    if api_key.is_empty() {
        eprintln!("No API key in response");
        return 1;
    }

    let cfg = json!({
        "apiBase": api,
        "apiKey": api_key,
        "displayName": user_name,
    });

    if let Err(e) = write_config(project_dir, &cfg) {
        eprintln!("Failed to write config: {e}");
        return 1;
    }

    println!("OK! Logged in successfully.");
    println!("  Name:     {user_name}");
    println!("  API Key:  {}...{}", &api_key[..8], &api_key[api_key.len()-4..]);
    println!("\nSync will now use this identity automatically.");

    sync_log(project_dir, &format!("Logged in: {user_name}"));
    0
}

fn cmd_login_with_key(project_dir: &Path, api_key: &str) -> i32 {
    let api = config_get(project_dir, "apiBase")
        .unwrap_or_else(|| "http://localhost:3001/api".to_string());

    let cfg = json!({
        "apiBase": api,
        "apiKey": api_key,
        "displayName": "user",
    });

    if let Err(e) = write_config(project_dir, &cfg) {
        eprintln!("Failed to write config: {e}");
        return 1;
    }

    println!("OK! API key saved.");
    println!("  API Key:  {}...{}", &api_key[..8.min(api_key.len())], &api_key[api_key.len().saturating_sub(4)..]);
    println!("\nSync will now use this key automatically.");

    sync_log(project_dir, "API key configured directly");
    0
}

fn cmd_status(project_dir: &Path) -> i32 {
    let cfg = match read_config(project_dir) {
        Some(c) => c,
        None => {
            println!("Not configured. Run: sync-tool login <email> <password>");
            return 1;
        }
    };

    let get = |k: &str| -> String {
        cfg.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };

    let key = get("apiKey");
    let key_display = if key.len() > 12 {
        format!("{}...{}", &key[..8], &key[key.len()-4..])
    } else {
        key
    };

    println!("Configured:");
    println!("  Name:     {}", get("displayName"));
    println!("  API Key:  {}", key_display);
    println!("  API:      {}", get("apiBase"));
    0
}

fn cmd_sync(project_dir: &Path) -> i32 {
    let api_key = match config_get(project_dir, "apiKey") {
        Some(key) => key,
        None => {
            sync_log(project_dir, "No API key configured — skipping sync");
            return 0;
        }
    };

    let api_base = config_get(project_dir, "apiBase")
        .unwrap_or_else(|| "http://localhost:3001/api".to_string());

    let mp = metrics_path(project_dir);
    match fs::metadata(&mp) {
        Ok(m) if m.len() > 0 => {}
        _ => {
            sync_log(project_dir, "No metrics to sync");
            return 0;
        }
    };

    // Lock file
    let lock = mp.with_extension("jsonl.lock");
    if lock.exists() {
        if let Ok(lm) = fs::metadata(&lock) {
            if let Ok(age) = lm
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .elapsed()
            {
                if age.as_secs() < 60 {
                    sync_log(project_dir, "Lock file exists, skipping sync");
                    return 0;
                }
            }
        }
        let _ = fs::remove_file(&lock);
        sync_log(project_dir, "Removed stale lock");
    }

    let _ = fs::write(&lock, format!("{}", std::process::id()));

    let sessions = aggregate_file(&mp);
    if sessions.is_empty() {
        sync_log(project_dir, "No sessions to sync");
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let payload = build_payload(&sessions);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let n = sessions.len();

    sync_log(project_dir, &format!("Syncing {n} sessions to {api_base}/sync"));

    // POST to /api/sync with API key header
    let url = format!("{api_base}/sync");
    let result = http_post(&url, &payload_str, Some(&api_key));

    let rc = match result {
        Ok((200, resp)) => {
            sync_log(project_dir, &format!("Sync OK: {resp}"));

            let ts = Utc::now().format("%Y%m%d_%H%M%S");
            let archive = project_dir.join(format!("metrics.synced.{ts}.jsonl"));
            let _ = fs::rename(&mp, &archive);
            let _ = fs::write(&mp, "");

            sync_log(
                project_dir,
                &format!(
                    "Flushed metrics.jsonl, archived to {}",
                    archive.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
            0
        }
        Ok((code, resp)) => {
            sync_log(project_dir, &format!("Sync FAILED (HTTP {code}): {resp}"));
            1
        }
        Err(e) => {
            sync_log(project_dir, &format!("Sync FAILED: {e}"));
            1
        }
    };

    let _ = fs::remove_file(&lock);
    rc
}

// ---- Main ----

fn main() {
    let args: Vec<String> = env::args().collect();
    let project_dir = resolve_project_dir();

    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  sync-tool log                            Log hook event from stdin");
        eprintln!("  sync-tool sync                           Sync metrics to backend");
        eprintln!("  sync-tool login <email> <password>       Login to get API key");
        eprintln!("  sync-tool login --api-key <key>          Set API key directly");
        eprintln!("  sync-tool status                         Show configuration");
        eprintln!("  sync-tool aggregate <file>               Output aggregated JSON");
        std::process::exit(1);
    }

    let rc = match args[1].as_str() {
        "log" => cmd_log(&project_dir),
        "sync" => cmd_sync(&project_dir),
        "login" => {
            if args.len() >= 4 && args[2] == "--api-key" {
                cmd_login_with_key(&project_dir, &args[3])
            } else if args.len() >= 4 {
                cmd_login_with_credentials(&project_dir, &args[2], &args[3])
            } else {
                eprintln!("Usage:");
                eprintln!("  sync-tool login <email> <password>");
                eprintln!("  sync-tool login --api-key <key>");
                1
            }
        }
        "status" => cmd_status(&project_dir),
        "aggregate" => {
            if args.len() < 3 {
                eprintln!("Usage: sync-tool aggregate <file>");
                std::process::exit(1);
            }
            cmd_aggregate(&args[2])
        }
        other => {
            eprintln!("Unknown command: {other}");
            1
        }
    };

    std::process::exit(rc);
}
