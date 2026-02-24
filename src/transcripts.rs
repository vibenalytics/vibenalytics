use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Seek};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use serde_json::Value;
use crate::hash::hash_path;
use crate::paths::cursors_path;
use crate::aggregation::Session;

// ---- Cursor state ----

pub fn read_cursors(dir: &Path) -> HashMap<String, Value> {
    let data = match fs::read_to_string(cursors_path(dir)) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    let val: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    match val {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

pub fn write_cursors(dir: &Path, cursors: &HashMap<String, Value>) {
    let map: serde_json::Map<String, Value> = cursors.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let val = Value::Object(map);
    let tmp = cursors_path(dir).with_extension("json.tmp");
    let target = cursors_path(dir);
    if let Ok(data) = serde_json::to_string_pretty(&val) {
        if fs::write(&tmp, &data).is_ok() {
            let _ = fs::rename(&tmp, &target);
        }
    }
}

// ---- Session discovery ----

/// Summary of a discovered project directory in ~/.claude/projects/
#[allow(dead_code)]
pub struct DiscoveredProject {
    pub dir_name: String,
    pub display_name: String,
    pub original_path: String,
    pub path_hash: String,
    pub session_count: usize,
    pub last_active: Option<SystemTime>,
}

/// Discover all project directories in ~/.claude/projects/ with session counts.
pub fn discover_projects(claude: &Path) -> Vec<DiscoveredProject> {
    let projects_dir = claude.join("projects");
    let mut results: Vec<DiscoveredProject> = Vec::new();

    let entries = match fs::read_dir(&projects_dir) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }

        let dir_name = project_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let original_path = dir_name.replacen('-', "/", 1).replace('-', "/");
        let display_name = original_path.rsplit('/').find(|s| !s.is_empty())
            .unwrap_or("unknown").to_string();
        let path_hash_val = hash_path(&original_path);

        let mut session_count = 0usize;
        let mut latest_modified: Option<SystemTime> = None;

        if let Ok(files) = fs::read_dir(&project_dir) {
            for f in files.flatten() {
                let p = f.path();
                if p.is_file() && p.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    session_count += 1;
                    if let Ok(meta) = fs::metadata(&p) {
                        if let Ok(modified) = meta.modified() {
                            latest_modified = Some(match latest_modified {
                                Some(prev) if modified > prev => modified,
                                Some(prev) => prev,
                                None => modified,
                            });
                        }
                    }
                }
            }
        }

        if session_count > 0 {
            results.push(DiscoveredProject {
                dir_name,
                display_name,
                original_path,
                path_hash: path_hash_val,
                session_count,
                last_active: latest_modified,
            });
        }
    }

    // Sort by last active descending (most recently used first)
    results.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    results
}

/// Discover all top-level session JSONL files in ~/.claude/projects/
/// Returns (project_name, path_hash, jsonl_path) tuples.
/// If `selected_dirs` is Some, only includes files from those directory names.
pub fn discover_sessions(claude: &Path, selected_dirs: Option<&std::collections::HashSet<String>>) -> Vec<(String, String, PathBuf)> {
    let projects_dir = claude.join("projects");
    let mut results: Vec<(String, String, PathBuf)> = Vec::new();

    let entries = match fs::read_dir(&projects_dir) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }

        let dir_name = project_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if let Some(selected) = selected_dirs {
            if !selected.contains(&dir_name) {
                continue;
            }
        }

        let original_path = dir_name.replacen('-', "/", 1).replace('-', "/");
        let project_name = original_path.rsplit('/').find(|s| !s.is_empty())
            .unwrap_or("unknown").to_string();
        let path_hash_val = hash_path(&original_path);

        if let Ok(files) = fs::read_dir(&project_dir) {
            for file in files.flatten() {
                let path = file.path();
                if path.is_file()
                    && path.extension().map(|e| e == "jsonl").unwrap_or(false)
                {
                    results.push((project_name.clone(), path_hash_val.clone(), path));
                }
            }
        }
    }

    results
}

// ---- Full transcript parsing ----

pub fn parse_session_transcript(filepath: &Path, fallback_project: &str, fallback_path_hash: &str) -> Option<Session> {
    let file = fs::File::open(filepath).ok()?;
    let reader = io::BufReader::new(file);

    let mut session = Session::new("unknown");
    session.project = fallback_project.to_string();
    // path_hash left empty — will be set from transcript cwd if available,
    // otherwise falls back to the directory-derived hash after parsing.

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let evt: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if session.session_id == "unknown" {
            if let Some(sid) = evt.get("sessionId").and_then(|v| v.as_str()) {
                session.session_id = sid.to_string();
            }
        }

        if let Some(ts) = evt.get("timestamp").and_then(|v| v.as_str()) {
            if session.started_at.is_empty() || ts < session.started_at.as_str() {
                session.started_at = ts.to_string();
            }
            if session.ended_at.is_empty() || ts > session.ended_at.as_str() {
                session.ended_at = ts.to_string();
            }
        }

        let msg_type = evt.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                session.message_count += 1;
                if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                            *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
            "user" => {
                session.message_count += 1;
                let is_prompt = evt
                    .pointer("/message/content")
                    .map(|v| v.is_string())
                    .unwrap_or(false);
                if is_prompt {
                    session.prompt_count += 1;
                }
                if let Some(pm) = evt.get("permissionMode").and_then(|v| v.as_str()) {
                    session.permission_mode = pm.to_string();
                }
                if session.project == fallback_project || session.project == "unknown" {
                    if let Some(cwd) = evt.get("cwd").and_then(|v| v.as_str()) {
                        if let Some(last) = cwd.rsplit('/').next() {
                            if !last.is_empty() {
                                session.project = last.to_string();
                            }
                        }
                        if session.path_hash.is_empty() {
                            session.path_hash = hash_path(cwd);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if session.session_id == "unknown" && session.message_count == 0 {
        return None;
    }

    // Fall back to directory-derived hash if transcript had no cwd
    if session.path_hash.is_empty() {
        session.path_hash = fallback_path_hash.to_string();
    }

    session.hostname = gethostname::gethostname()
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string();

    Some(session)
}

// ---- Incremental transcript parsing ----

struct UsageAccum {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

pub fn parse_transcript_from_offset(
    filepath: &Path,
    byte_offset: u64,
    prev_request_id: &str,
    prev_output_tokens: u64,
    fallback_project: &str,
    fallback_path_hash: &str,
) -> Option<(Session, u64, String, u64)> {
    let file_size = fs::metadata(filepath).ok()?.len();
    let start_offset = if file_size < byte_offset { 0 } else { byte_offset };

    let mut file = fs::File::open(filepath).ok()?;
    file.seek(io::SeekFrom::Start(start_offset)).ok()?;
    let reader = io::BufReader::new(file);

    let mut session = Session::new("unknown");
    session.project = fallback_project.to_string();
    // path_hash left empty — will be set from transcript cwd if available,
    // otherwise falls back to the directory-derived hash after parsing.

    let mut usage_map: HashMap<String, UsageAccum> = HashMap::new();
    let mut last_request_id = String::new();
    let mut current_offset = start_offset;
    let mut lines_parsed = 0u32;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        let line_bytes = line.len() as u64 + 1;
        current_offset += line_bytes;

        if line.trim().is_empty() {
            continue;
        }

        let evt: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed JSON, keep reading
        };

        lines_parsed += 1;

        let msg_type = evt.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if session.session_id == "unknown" {
            if let Some(sid) = evt.get("sessionId").and_then(|v| v.as_str()) {
                session.session_id = sid.to_string();
            }
        }

        if let Some(ts) = evt.get("timestamp").and_then(|v| v.as_str()) {
            if session.started_at.is_empty() || ts < session.started_at.as_str() {
                session.started_at = ts.to_string();
            }
            if session.ended_at.is_empty() || ts > session.ended_at.as_str() {
                session.ended_at = ts.to_string();
            }
        }

        match msg_type {
            "assistant" => {
                session.message_count += 1;
                if let Some(model) = evt.pointer("/message/model").and_then(|v| v.as_str()) {
                    if session.model.is_empty() || !model.is_empty() {
                        session.model = model.to_string();
                    }
                }
                let request_id = evt.get("requestId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| evt.get("uuid").and_then(|v| v.as_str()))
                    .unwrap_or("unknown")
                    .to_string();
                if let Some(usage) = evt.pointer("/message/usage") {
                    let out_tok = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let entry = usage_map.entry(request_id.clone()).or_insert_with(|| UsageAccum {
                        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        cache_read_tokens: usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        cache_creation_tokens: usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        output_tokens: 0,
                    });
                    if out_tok > entry.output_tokens {
                        entry.output_tokens = out_tok;
                    }
                }
                if !request_id.is_empty() {
                    last_request_id = request_id;
                }
                if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                            *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
            "user" => {
                session.message_count += 1;
                let is_prompt = evt
                    .pointer("/message/content")
                    .map(|v| v.is_string())
                    .unwrap_or(false);
                if is_prompt {
                    session.prompt_count += 1;
                }
                if let Some(pm) = evt.get("permissionMode").and_then(|v| v.as_str()) {
                    session.permission_mode = pm.to_string();
                }
                if session.project == fallback_project || session.project == "unknown" {
                    if let Some(cwd) = evt.get("cwd").and_then(|v| v.as_str()) {
                        if let Some(last) = cwd.rsplit('/').next() {
                            if !last.is_empty() {
                                session.project = last.to_string();
                            }
                        }
                        if session.path_hash.is_empty() {
                            session.path_hash = hash_path(cwd);
                        }
                    }
                }
            }
            "system" => {
                let subtype = evt.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
                if subtype == "turn_duration" {
                    if let Some(ms) = evt.get("durationMs").and_then(|v| v.as_u64()) {
                        session.total_turn_duration_ms += ms;
                        session.turn_count += 1;
                    }
                }
            }
            _ => {}
        }
    }

    if lines_parsed == 0 {
        return None;
    }

    // Fall back to directory-derived hash if transcript had no cwd
    if session.path_hash.is_empty() {
        session.path_hash = fallback_path_hash.to_string();
    }

    for (rid, accum) in &usage_map {
        let mut out = accum.output_tokens;
        if rid == prev_request_id && !prev_request_id.is_empty() {
            if out > prev_output_tokens {
                out -= prev_output_tokens;
            } else {
                out = 0;
            }
        } else {
            session.total_input_tokens += accum.input_tokens;
            session.total_cache_read_tokens += accum.cache_read_tokens;
            session.total_cache_creation_tokens += accum.cache_creation_tokens;
        }
        session.total_output_tokens += out;
    }

    session.hostname = gethostname::gethostname()
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let last_out = usage_map
        .get(&last_request_id)
        .map(|a| a.output_tokens)
        .unwrap_or(0);

    Some((session, current_offset, last_request_id, last_out))
}
