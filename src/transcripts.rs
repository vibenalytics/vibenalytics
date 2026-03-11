use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Seek};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use serde_json::Value;
use crate::hash::hash_path;
use crate::paths::cursors_path;
use crate::aggregation::{Session, RequestUsage, PromptUsage, classify_prompt};

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

/// Recursively collect all .jsonl files under a directory.
fn collect_jsonl_recursive(dir: &Path, results: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_recursive(&path, results);
        } else if path.is_file() && path.extension().map(|e| e == "jsonl").unwrap_or(false) {
            results.push(path);
        }
    }
}

/// Discover all session JSONL files in ~/.claude/projects/ (including subagent files).
/// Returns (project_name, path_hash, jsonl_path, is_subagent, parent_session_id) tuples.
/// If `selected_dirs` is Some, only includes files from those directory names.
pub fn discover_sessions(claude: &Path, selected_dirs: Option<&std::collections::HashSet<String>>) -> Vec<(String, String, PathBuf, bool, String)> {
    let projects_dir = claude.join("projects");
    let mut results: Vec<(String, String, PathBuf, bool, String)> = Vec::new();

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

        let mut all_jsonl = Vec::new();
        collect_jsonl_recursive(&project_dir, &mut all_jsonl);

        for path in all_jsonl {
            let path_str = path.to_string_lossy();
            let is_subagent = path_str.contains("/subagents/agent-");
            let parent_session_id = if is_subagent {
                // Extract parent session ID: the directory name above `subagents/`
                extract_parent_session_id(&path_str)
            } else {
                String::new()
            };
            results.push((project_name.clone(), path_hash_val.clone(), path, is_subagent, parent_session_id));
        }
    }

    results
}

/// Extract parent session ID from a subagent path.
/// Path pattern: .../{session_id}/subagents/agent-{N}/{file}.jsonl
fn extract_parent_session_id(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == "subagents" && i > 0 {
            return parts[i - 1].to_string();
        }
    }
    String::new()
}

/// Find subagent JSONL files for a given parent session transcript path.
/// Looks for {session_dir}/{session_id}/subagents/agent-*/... .jsonl files.
pub fn find_subagent_files(parent_transcript: &Path) -> Vec<PathBuf> {
    let session_id = parent_transcript
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let parent_dir = match parent_transcript.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let subagents_dir = parent_dir.join(&session_id).join("subagents");
    if !subagents_dir.is_dir() {
        return Vec::new();
    }
    let mut results = Vec::new();
    collect_jsonl_recursive(&subagents_dir, &mut results);
    // Filter out compact subagent files — these contain pre-compaction data
    // that already exists in the parent transcript, so including them would
    // double-count tokens and line changes.
    results.retain(|p| {
        let name = p.file_stem().unwrap_or_default().to_string_lossy();
        !name.starts_with("agent-acompact")
    });
    results
}

// ---- Prompt detection ----

/// Returns true if a user event represents a real typed prompt (not system noise).
fn is_real_user_prompt(content: &str) -> bool {
    !content.is_empty()
        && !content.starts_with("<local-command")
        && !content.starts_with("<bash-")
        && !content.starts_with("/plugin")
}

/// Extract file extension from a path (e.g. "/foo/bar.rs" -> "rs", "Makefile" -> "").
fn extract_extension(path: &str) -> String {
    match path.rsplit('/').next().unwrap_or(path).rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 10 => ext.to_lowercase(),
        _ => String::new(),
    }
}

// ---- Patch line counting ----

/// Count added/removed lines from a structuredPatch array.
/// Each hunk has a "lines" array with "+"/"-"/" " prefixed strings.
fn count_patch_lines(patches: &[Value]) -> (u64, u64) {
    let mut added = 0u64;
    let mut removed = 0u64;
    for hunk in patches {
        if let Some(lines) = hunk.get("lines").and_then(|v| v.as_array()) {
            for line in lines {
                if let Some(s) = line.as_str() {
                    if s.starts_with('+') {
                        added += 1;
                    } else if s.starts_with('-') {
                        removed += 1;
                    }
                }
            }
        }
    }
    (added, removed)
}

// ---- Full transcript parsing ----

struct FullParseAccum {
    message_id: String,
    request_id: String,
    timestamp: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    seen_tool_use_ids: HashSet<String>,
    prompt_index: i32,
    tools: HashMap<String, u32>,
    lines_added: u64,
    lines_removed: u64,
    lines_by_extension: HashMap<String, (u64, u64)>,
}

pub fn parse_session_transcript(filepath: &Path, fallback_project: &str, fallback_path_hash: &str) -> Option<Session> {
    let file = fs::File::open(filepath).ok()?;
    let reader = io::BufReader::new(file);

    let mut session = Session::new("unknown");
    session.project = fallback_project.to_string();

    let mut accum_map: HashMap<String, FullParseAccum> = HashMap::new();
    let mut tool_use_to_request: HashMap<String, String> = HashMap::new();
    let mut tool_use_to_extension: HashMap<String, String> = HashMap::new();
    let mut write_lines_by_tool_id: HashMap<String, u64> = HashMap::new();
    let mut edit_lines_by_tool_id: HashMap<String, (u64, u64)> = HashMap::new(); // (added, removed) from old_string/new_string
    let mut seen_user_messages: HashSet<String> = HashSet::new();
    let mut current_prompt_index: i32 = -1;
    let mut current_prompt_tools: HashMap<String, u32> = HashMap::new();
    let mut current_prompt_ts = String::new();
    let mut current_prompt_length: usize = 0;
    let mut current_prompt_type = String::new();
    let mut current_prompt_command = String::new();
    let mut current_prompt_skills: Vec<String> = Vec::new();
    let mut pending_compaction: Option<(String, u64)> = None; // (trigger, pre_tokens)

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

        if let Some(v) = evt.get("version").and_then(|v| v.as_str()) {
            if session.claude_version.is_empty() || v > session.claude_version.as_str() {
                session.claude_version = v.to_string();
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
                let model = evt.pointer("/message/model")
                    .and_then(|v| v.as_str()).unwrap_or("");
                // Skip synthetic messages
                if model == "<synthetic>" {
                    continue;
                }

                let message_id = evt.pointer("/message/id")
                    .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                let request_id = evt.get("requestId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| evt.pointer("/message/id").and_then(|v| v.as_str()))
                    .unwrap_or("unknown")
                    .to_string();
                let ts = evt.get("timestamp")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();

                let key = format!("{}:{}", message_id, request_id);
                let is_new = !accum_map.contains_key(&key);
                if is_new {
                    session.message_count += 1;
                }

                let key_clone = key.clone();
                let accum = accum_map.entry(key).or_insert_with(|| FullParseAccum {
                    message_id: message_id.clone(),
                    request_id: request_id.clone(),
                    timestamp: ts.clone(),
                    model: model.to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    seen_tool_use_ids: HashSet::new(),
                    prompt_index: current_prompt_index.max(0),
                    tools: HashMap::new(),
                    lines_added: 0,
                    lines_removed: 0,
                    lines_by_extension: HashMap::new(),
                });

                if !model.is_empty() {
                    accum.model = model.to_string();
                    session.model = model.to_string();
                }

                if let Some(usage) = evt.pointer("/message/usage") {
                    let input = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cache_create = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    if is_new {
                        accum.input_tokens = input;
                        accum.cache_read_tokens = cache_read;
                        accum.cache_creation_tokens = cache_create;
                    }
                    if output > accum.output_tokens {
                        accum.output_tokens = output;
                    }
                }

                if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            if !tool_id.is_empty() && accum.seen_tool_use_ids.insert(tool_id.to_string()) {
                                let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                                *current_prompt_tools.entry(tool.to_string()).or_insert(0) += 1;
                                *accum.tools.entry(tool.to_string()).or_insert(0) += 1;
                                tool_use_to_request.insert(tool_id.to_string(), key_clone.clone());
                                // Track file extension for Write/Edit tools
                                if tool == "Write" || tool == "Edit" {
                                    if let Some(fp) = block.pointer("/input/file_path").and_then(|v| v.as_str()) {
                                        let ext = extract_extension(fp);
                                        if !ext.is_empty() {
                                            tool_use_to_extension.insert(tool_id.to_string(), ext);
                                        }
                                    }
                                }
                                // For Write tool, count lines in the content being written
                                if tool == "Write" {
                                    if let Some(file_content) = block.pointer("/input/content").and_then(|v| v.as_str()) {
                                        let line_count = if file_content.is_empty() { 0 } else {
                                            file_content.lines().count() as u64
                                        };
                                        if line_count > 0 {
                                            write_lines_by_tool_id.insert(tool_id.to_string(), line_count);
                                        }
                                    }
                                }
                                // For Edit tool, store old_string/new_string line counts as fallback
                                // (subagent transcripts lack structuredPatch in tool results)
                                if tool == "Edit" {
                                    let old_lines = block.pointer("/input/old_string")
                                        .and_then(|v| v.as_str())
                                        .map(|s| if s.is_empty() { 0 } else { s.lines().count() as u64 })
                                        .unwrap_or(0);
                                    let new_lines = block.pointer("/input/new_string")
                                        .and_then(|v| v.as_str())
                                        .map(|s| if s.is_empty() { 0 } else { s.lines().count() as u64 })
                                        .unwrap_or(0);
                                    if new_lines > 0 || old_lines > 0 {
                                        edit_lines_by_tool_id.insert(tool_id.to_string(), (new_lines, old_lines));
                                    }
                                }
                                // Extract skill name from Skill tool invocations
                                if tool == "Skill" {
                                    if let Some(skill_name) = block.pointer("/input/skill").and_then(|v| v.as_str()) {
                                        let formatted = format!("/{}", skill_name);
                                        if !current_prompt_skills.contains(&formatted) {
                                            current_prompt_skills.push(formatted);
                                        }
                                    }
                                }
                            } else if tool_id.is_empty() {
                                let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                                *current_prompt_tools.entry(tool.to_string()).or_insert(0) += 1;
                                *accum.tools.entry(tool.to_string()).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
            "user" => {
                let user_msg_id = evt.get("uuid")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !user_msg_id.is_empty() && !seen_user_messages.insert(user_msg_id) {
                    continue; // duplicate user message
                }
                // Compaction summary: flush current prompt and start a compaction entry
                let is_compact_summary = evt.get("isCompactSummary")
                    .and_then(|v| v.as_bool()).unwrap_or(false);
                if is_compact_summary {
                    // Flush current prompt before compaction
                    if current_prompt_index >= 0 {
                        session.prompts.push(PromptUsage {
                            prompt_index: current_prompt_index,
                            timestamp: current_prompt_ts.clone(),
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_creation_tokens: 0,
                            request_count: 0,
                            tools: std::mem::take(&mut current_prompt_tools),
                            model: String::new(),
                            prompt_length: std::mem::replace(&mut current_prompt_length, 0),
                            msg_type: std::mem::take(&mut current_prompt_type),
                            command: std::mem::take(&mut current_prompt_command),
                            skills: std::mem::take(&mut current_prompt_skills),
                            subagent_count: 0,
                            compaction_trigger: String::new(),
                            compaction_pre_tokens: 0,
                            context_tokens: 0,
                        });
                    }
                    current_prompt_index += 1;
                    current_prompt_tools.clear();
                    current_prompt_ts = evt.get("timestamp")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    current_prompt_length = 0;
                    current_prompt_type = "compaction".to_string();
                    current_prompt_command = String::new();
                    current_prompt_skills = Vec::new();
                    // Attach compaction metadata from pending system event
                    if let Some((trigger, pre_tokens)) = pending_compaction.take() {
                        session.prompts.push(PromptUsage {
                            prompt_index: current_prompt_index,
                            timestamp: current_prompt_ts.clone(),
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_creation_tokens: 0,
                            request_count: 0,
                            tools: HashMap::new(),
                            model: String::new(),
                            prompt_length: 0,
                            msg_type: "compaction".to_string(),
                            command: String::new(),
                            skills: Vec::new(),
                            subagent_count: 0,
                            compaction_trigger: trigger,
                            compaction_pre_tokens: pre_tokens,
                            context_tokens: 0,
                        });
                    }
                    continue;
                }
                session.message_count += 1;
                let content_str = evt.pointer("/message/content")
                    .and_then(|v| v.as_str());
                if let Some(text) = content_str {
                    if is_real_user_prompt(text) {
                        // Save previous prompt before starting new one
                        if current_prompt_index >= 0 {
                            session.prompts.push(PromptUsage {
                                prompt_index: current_prompt_index,
                                timestamp: current_prompt_ts.clone(),
                                input_tokens: 0,
                                output_tokens: 0,
                                cache_read_tokens: 0,
                                cache_creation_tokens: 0,
                                request_count: 0,
                                tools: std::mem::take(&mut current_prompt_tools),
                                model: String::new(),
                                prompt_length: std::mem::replace(&mut current_prompt_length, 0),
                                msg_type: std::mem::take(&mut current_prompt_type),
                                command: std::mem::take(&mut current_prompt_command),
                                skills: std::mem::take(&mut current_prompt_skills),
                                subagent_count: 0,
                                compaction_trigger: String::new(),
                                compaction_pre_tokens: 0,
                                context_tokens: 0,
                            });
                        }
                        current_prompt_index += 1;
                        current_prompt_tools.clear();
                        current_prompt_ts = evt.get("timestamp")
                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                        current_prompt_length = text.len();
                        let (pt, pc, ps) = classify_prompt(text);
                        current_prompt_type = pt;
                        current_prompt_command = pc;
                        current_prompt_skills = ps;
                        session.prompt_count += 1;
                    }
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
                // Extract lines added/removed from toolUseResult.structuredPatch
                if let Some(patches) = evt.pointer("/toolUseResult/structuredPatch").and_then(|v| v.as_array()) {
                    let (added, removed) = count_patch_lines(patches);
                    if added > 0 || removed > 0 {
                        // Find the tool_use_id from message.content to link back to the request
                        let tool_use_id = evt.pointer("/message/content")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.iter().find_map(|b| {
                                b.get("tool_use_id").and_then(|v| v.as_str())
                            }));
                        if let Some(tui) = tool_use_id {
                            if let Some(req_key) = tool_use_to_request.get(tui) {
                                if let Some(accum) = accum_map.get_mut(req_key) {
                                    accum.lines_added += added;
                                    accum.lines_removed += removed;
                                    if let Some(ext) = tool_use_to_extension.get(tui) {
                                        let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                        entry.0 += added;
                                        entry.1 += removed;
                                    }
                                }
                            }
                        }
                    }
                }
                // For new file creation (Write with empty structuredPatch), count lines from tool input.
                // Also apply Edit fallback when structuredPatch is absent (e.g. subagent transcripts).
                let has_patch = evt.pointer("/toolUseResult/structuredPatch")
                    .and_then(|v| v.as_array())
                    .map_or(false, |a| !a.is_empty());
                if !has_patch {
                    if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                        for block in content {
                            if let Some(tui) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                                if let Some(wlines) = write_lines_by_tool_id.remove(tui) {
                                    if let Some(req_key) = tool_use_to_request.get(tui) {
                                        if let Some(accum) = accum_map.get_mut(req_key) {
                                            accum.lines_added += wlines;
                                            if let Some(ext) = tool_use_to_extension.get(tui) {
                                                let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                                entry.0 += wlines;
                                            }
                                        }
                                    }
                                }
                                // Edit fallback: use old_string/new_string line counts
                                if let Some((added, removed)) = edit_lines_by_tool_id.remove(tui) {
                                    if let Some(req_key) = tool_use_to_request.get(tui) {
                                        if let Some(accum) = accum_map.get_mut(req_key) {
                                            accum.lines_added += added;
                                            accum.lines_removed += removed;
                                            if let Some(ext) = tool_use_to_extension.get(tui) {
                                                let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                                entry.0 += added;
                                                entry.1 += removed;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "system" => {
                let subtype = evt.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
                if subtype == "compact_boundary" {
                    let trigger = evt.pointer("/compactMetadata/trigger")
                        .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let pre_tokens = evt.pointer("/compactMetadata/preTokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                    pending_compaction = Some((trigger, pre_tokens));
                }
            }
            _ => {}
        }
    }

    // Flush the last prompt
    if current_prompt_index >= 0 {
        session.prompts.push(PromptUsage {
            prompt_index: current_prompt_index,
            timestamp: current_prompt_ts,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_count: 0,
            tools: current_prompt_tools,
            model: String::new(),
            prompt_length: current_prompt_length,
            msg_type: current_prompt_type,
            command: current_prompt_command,
            skills: current_prompt_skills,
            subagent_count: 0,
            compaction_trigger: String::new(),
            compaction_pre_tokens: 0,
            context_tokens: 0,
        });
    }

    if session.session_id == "unknown" && session.message_count == 0 {
        return None;
    }

    if session.path_hash.is_empty() {
        session.path_hash = fallback_path_hash.to_string();
    }

    // Convert accumulators to RequestUsage vec + cumulative totals
    for accum in accum_map.into_values() {
        session.total_input_tokens += accum.input_tokens;
        session.total_output_tokens += accum.output_tokens;
        session.total_cache_read_tokens += accum.cache_read_tokens;
        session.total_cache_creation_tokens += accum.cache_creation_tokens;
        session.total_lines_added += accum.lines_added;
        session.total_lines_removed += accum.lines_removed;
        crate::aggregation::merge_extension_lines(&mut session.total_lines_by_extension, &accum.lines_by_extension);
        session.requests.push(RequestUsage {
            request_id: accum.request_id,
            message_id: accum.message_id,
            timestamp: accum.timestamp,
            model: accum.model,
            input_tokens: accum.input_tokens,
            output_tokens: accum.output_tokens,
            cache_read_tokens: accum.cache_read_tokens,
            cache_creation_tokens: accum.cache_creation_tokens,
            is_subagent: false,
            prompt_index: accum.prompt_index,
            tools: accum.tools,
            lines_added: accum.lines_added,
            lines_removed: accum.lines_removed,
            lines_by_extension: accum.lines_by_extension,
        });
    }

    Some(session)
}

// ---- Incremental transcript parsing ----

struct RequestAccum {
    request_id: String,
    message_id: String,
    timestamp: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    seen_tool_use_ids: HashSet<String>,
    prompt_index: i32,
    tools: HashMap<String, u32>,
    lines_added: u64,
    lines_removed: u64,
    lines_by_extension: HashMap<String, (u64, u64)>,
}

/// Returns (Session, new_byte_offset, last_request_id, last_message_id, last_output_tokens).
pub fn parse_transcript_from_offset(
    filepath: &Path,
    byte_offset: u64,
    prev_request_id: &str,
    prev_message_id: &str,
    prev_output_tokens: u64,
    fallback_project: &str,
    fallback_path_hash: &str,
    prompt_index_offset: i32,
) -> Option<(Session, u64, String, String, u64)> {
    let file_size = fs::metadata(filepath).ok()?.len();
    let start_offset = if file_size < byte_offset { 0 } else { byte_offset };

    let mut file = fs::File::open(filepath).ok()?;
    file.seek(io::SeekFrom::Start(start_offset)).ok()?;
    let reader = io::BufReader::new(file);

    let mut session = Session::new("unknown");
    session.project = fallback_project.to_string();

    let mut accum_map: HashMap<String, RequestAccum> = HashMap::new();
    let mut tool_use_to_request: HashMap<String, String> = HashMap::new();
    let mut tool_use_to_extension: HashMap<String, String> = HashMap::new();
    let mut write_lines_by_tool_id: HashMap<String, u64> = HashMap::new();
    let mut edit_lines_by_tool_id: HashMap<String, (u64, u64)> = HashMap::new(); // (added, removed) from old_string/new_string
    let mut last_request_id = String::new();
    let mut last_message_id = String::new();
    let mut current_offset = start_offset;
    let mut lines_parsed = 0u32;
    let mut seen_user_messages: HashSet<String> = HashSet::new();
    let mut current_prompt_index: i32 = prompt_index_offset - 1;
    let mut current_prompt_tools: HashMap<String, u32> = HashMap::new();
    let mut current_prompt_ts = String::new();
    let mut current_prompt_length: usize = 0;
    let mut current_prompt_type = String::new();
    let mut current_prompt_command = String::new();
    let mut current_prompt_skills: Vec<String> = Vec::new();
    let mut pending_compaction: Option<(String, u64)> = None; // (trigger, pre_tokens)
    let mut prompt_started = false; // true after first real user message or compaction

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
            Err(_) => continue,
        };

        lines_parsed += 1;

        let msg_type = evt.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if session.session_id == "unknown" {
            if let Some(sid) = evt.get("sessionId").and_then(|v| v.as_str()) {
                session.session_id = sid.to_string();
            }
        }

        if let Some(v) = evt.get("version").and_then(|v| v.as_str()) {
            if session.claude_version.is_empty() || v > session.claude_version.as_str() {
                session.claude_version = v.to_string();
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
                let model = evt.pointer("/message/model")
                    .and_then(|v| v.as_str()).unwrap_or("");
                if model == "<synthetic>" {
                    continue;
                }

                if !model.is_empty() {
                    session.model = model.to_string();
                }

                let message_id = evt.pointer("/message/id")
                    .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                let request_id = evt.get("requestId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| evt.pointer("/message/id").and_then(|v| v.as_str()))
                    .unwrap_or("unknown")
                    .to_string();
                let ts = evt.get("timestamp")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();

                let key = format!("{}:{}", message_id, request_id);
                let is_new = !accum_map.contains_key(&key);
                if is_new {
                    session.message_count += 1;
                }

                let key_clone = key.clone();
                let accum = accum_map.entry(key).or_insert_with(|| RequestAccum {
                    request_id: request_id.clone(),
                    message_id: message_id.clone(),
                    timestamp: ts.clone(),
                    model: model.to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    seen_tool_use_ids: HashSet::new(),
                    prompt_index: current_prompt_index.max(0),
                    tools: HashMap::new(),
                    lines_added: 0,
                    lines_removed: 0,
                    lines_by_extension: HashMap::new(),
                });

                if !model.is_empty() {
                    accum.model = model.to_string();
                }

                if let Some(usage) = evt.pointer("/message/usage") {
                    let input = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cache_create = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    if is_new {
                        accum.input_tokens = input;
                        accum.cache_read_tokens = cache_read;
                        accum.cache_creation_tokens = cache_create;
                    }
                    if output > accum.output_tokens {
                        accum.output_tokens = output;
                    }
                }

                if !request_id.is_empty() {
                    last_request_id = request_id;
                }
                if !message_id.is_empty() {
                    last_message_id = message_id;
                }

                if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            if !tool_id.is_empty() && accum.seen_tool_use_ids.insert(tool_id.to_string()) {
                                let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                                *current_prompt_tools.entry(tool.to_string()).or_insert(0) += 1;
                                *accum.tools.entry(tool.to_string()).or_insert(0) += 1;
                                tool_use_to_request.insert(tool_id.to_string(), key_clone.clone());
                                // Track file extension for Write/Edit tools
                                if tool == "Write" || tool == "Edit" {
                                    if let Some(fp) = block.pointer("/input/file_path").and_then(|v| v.as_str()) {
                                        let ext = extract_extension(fp);
                                        if !ext.is_empty() {
                                            tool_use_to_extension.insert(tool_id.to_string(), ext);
                                        }
                                    }
                                }
                                // For Write tool, count lines in the content being written
                                if tool == "Write" {
                                    if let Some(file_content) = block.pointer("/input/content").and_then(|v| v.as_str()) {
                                        let line_count = if file_content.is_empty() { 0 } else {
                                            file_content.lines().count() as u64
                                        };
                                        if line_count > 0 {
                                            write_lines_by_tool_id.insert(tool_id.to_string(), line_count);
                                        }
                                    }
                                }
                                // For Edit tool, store old_string/new_string line counts as fallback
                                // (subagent transcripts lack structuredPatch in tool results)
                                if tool == "Edit" {
                                    let old_lines = block.pointer("/input/old_string")
                                        .and_then(|v| v.as_str())
                                        .map(|s| if s.is_empty() { 0 } else { s.lines().count() as u64 })
                                        .unwrap_or(0);
                                    let new_lines = block.pointer("/input/new_string")
                                        .and_then(|v| v.as_str())
                                        .map(|s| if s.is_empty() { 0 } else { s.lines().count() as u64 })
                                        .unwrap_or(0);
                                    if new_lines > 0 || old_lines > 0 {
                                        edit_lines_by_tool_id.insert(tool_id.to_string(), (new_lines, old_lines));
                                    }
                                }
                                // Extract skill name from Skill tool invocations
                                if tool == "Skill" {
                                    if let Some(skill_name) = block.pointer("/input/skill").and_then(|v| v.as_str()) {
                                        let formatted = format!("/{}", skill_name);
                                        if !current_prompt_skills.contains(&formatted) {
                                            current_prompt_skills.push(formatted);
                                        }
                                    }
                                }
                            } else if tool_id.is_empty() {
                                let tool = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                *session.tools.entry(tool.to_string()).or_insert(0) += 1;
                                *current_prompt_tools.entry(tool.to_string()).or_insert(0) += 1;
                                *accum.tools.entry(tool.to_string()).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
            "user" => {
                let user_msg_id = evt.get("uuid")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !user_msg_id.is_empty() && !seen_user_messages.insert(user_msg_id) {
                    continue;
                }
                // Compaction summary: flush current prompt and start a compaction entry
                let is_compact_summary = evt.get("isCompactSummary")
                    .and_then(|v| v.as_bool()).unwrap_or(false);
                if is_compact_summary {
                    if prompt_started && current_prompt_index >= 0 {
                        session.prompts.push(PromptUsage {
                            prompt_index: current_prompt_index,
                            timestamp: current_prompt_ts.clone(),
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_creation_tokens: 0,
                            request_count: 0,
                            tools: std::mem::take(&mut current_prompt_tools),
                            model: String::new(),
                            prompt_length: std::mem::replace(&mut current_prompt_length, 0),
                            msg_type: std::mem::take(&mut current_prompt_type),
                            command: std::mem::take(&mut current_prompt_command),
                            skills: std::mem::take(&mut current_prompt_skills),
                            subagent_count: 0,
                            compaction_trigger: String::new(),
                            compaction_pre_tokens: 0,
                            context_tokens: 0,
                        });
                    }
                    prompt_started = true;
                    current_prompt_index += 1;
                    current_prompt_tools.clear();
                    current_prompt_ts = evt.get("timestamp")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    current_prompt_length = 0;
                    current_prompt_type = "compaction".to_string();
                    current_prompt_command = String::new();
                    current_prompt_skills = Vec::new();
                    if let Some((trigger, pre_tokens)) = pending_compaction.take() {
                        session.prompts.push(PromptUsage {
                            prompt_index: current_prompt_index,
                            timestamp: current_prompt_ts.clone(),
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_creation_tokens: 0,
                            request_count: 0,
                            tools: HashMap::new(),
                            model: String::new(),
                            prompt_length: 0,
                            msg_type: "compaction".to_string(),
                            command: String::new(),
                            skills: Vec::new(),
                            subagent_count: 0,
                            compaction_trigger: trigger,
                            compaction_pre_tokens: pre_tokens,
                            context_tokens: 0,
                        });
                    }
                    continue;
                }
                session.message_count += 1;
                let content_str = evt.pointer("/message/content")
                    .and_then(|v| v.as_str());
                if let Some(text) = content_str {
                    if is_real_user_prompt(text) {
                        if prompt_started && current_prompt_index >= 0 {
                            session.prompts.push(PromptUsage {
                                prompt_index: current_prompt_index,
                                timestamp: current_prompt_ts.clone(),
                                input_tokens: 0,
                                output_tokens: 0,
                                cache_read_tokens: 0,
                                cache_creation_tokens: 0,
                                request_count: 0,
                                tools: std::mem::take(&mut current_prompt_tools),
                                model: String::new(),
                                prompt_length: std::mem::replace(&mut current_prompt_length, 0),
                                msg_type: std::mem::take(&mut current_prompt_type),
                                command: std::mem::take(&mut current_prompt_command),
                                skills: std::mem::take(&mut current_prompt_skills),
                                subagent_count: 0,
                                compaction_trigger: String::new(),
                                compaction_pre_tokens: 0,
                                context_tokens: 0,
                            });
                        }
                        prompt_started = true;
                        current_prompt_index += 1;
                        current_prompt_tools.clear();
                        current_prompt_ts = evt.get("timestamp")
                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                        current_prompt_length = text.len();
                        let (pt, pc, ps) = classify_prompt(text);
                        current_prompt_type = pt;
                        current_prompt_command = pc;
                        current_prompt_skills = ps;
                        session.prompt_count += 1;
                    }
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
                // Extract lines added/removed from toolUseResult.structuredPatch
                if let Some(patches) = evt.pointer("/toolUseResult/structuredPatch").and_then(|v| v.as_array()) {
                    let (added, removed) = count_patch_lines(patches);
                    if added > 0 || removed > 0 {
                        let tool_use_id = evt.pointer("/message/content")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.iter().find_map(|b| {
                                b.get("tool_use_id").and_then(|v| v.as_str())
                            }));
                        if let Some(tui) = tool_use_id {
                            if let Some(req_key) = tool_use_to_request.get(tui) {
                                if let Some(accum) = accum_map.get_mut(req_key) {
                                    accum.lines_added += added;
                                    accum.lines_removed += removed;
                                    if let Some(ext) = tool_use_to_extension.get(tui) {
                                        let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                        entry.0 += added;
                                        entry.1 += removed;
                                    }
                                }
                            }
                        }
                    }
                }
                // For new file creation (Write with empty structuredPatch), count lines from tool input.
                // Also apply Edit fallback when structuredPatch is absent (e.g. subagent transcripts).
                let has_patch = evt.pointer("/toolUseResult/structuredPatch")
                    .and_then(|v| v.as_array())
                    .map_or(false, |a| !a.is_empty());
                if !has_patch {
                    if let Some(content) = evt.pointer("/message/content").and_then(|v| v.as_array()) {
                        for block in content {
                            if let Some(tui) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                                if let Some(wlines) = write_lines_by_tool_id.remove(tui) {
                                    if let Some(req_key) = tool_use_to_request.get(tui) {
                                        if let Some(accum) = accum_map.get_mut(req_key) {
                                            accum.lines_added += wlines;
                                            if let Some(ext) = tool_use_to_extension.get(tui) {
                                                let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                                entry.0 += wlines;
                                            }
                                        }
                                    }
                                }
                                // Edit fallback: use old_string/new_string line counts
                                if let Some((added, removed)) = edit_lines_by_tool_id.remove(tui) {
                                    if let Some(req_key) = tool_use_to_request.get(tui) {
                                        if let Some(accum) = accum_map.get_mut(req_key) {
                                            accum.lines_added += added;
                                            accum.lines_removed += removed;
                                            if let Some(ext) = tool_use_to_extension.get(tui) {
                                                let entry = accum.lines_by_extension.entry(ext.clone()).or_insert((0, 0));
                                                entry.0 += added;
                                                entry.1 += removed;
                                            }
                                        }
                                    }
                                }
                            }
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
                if subtype == "compact_boundary" {
                    let trigger = evt.pointer("/compactMetadata/trigger")
                        .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let pre_tokens = evt.pointer("/compactMetadata/preTokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                    pending_compaction = Some((trigger, pre_tokens));
                }
            }
            _ => {}
        }
    }

    // Flush the last prompt (only if a real user message or compaction was seen)
    if prompt_started && current_prompt_index >= 0 {
        session.prompts.push(PromptUsage {
            prompt_index: current_prompt_index,
            timestamp: current_prompt_ts,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_count: 0,
            tools: current_prompt_tools,
            model: String::new(),
            prompt_length: current_prompt_length,
            msg_type: current_prompt_type,
            command: current_prompt_command,
            skills: current_prompt_skills,
            subagent_count: 0,
            compaction_trigger: String::new(),
            compaction_pre_tokens: 0,
            context_tokens: 0,
        });
    }

    if lines_parsed == 0 {
        return None;
    }

    if session.path_hash.is_empty() {
        session.path_hash = fallback_path_hash.to_string();
    }

    // Boundary match: determine if a key matches the previous cursor position
    let prev_composite = if !prev_message_id.is_empty() {
        format!("{}:{}", prev_message_id, prev_request_id)
    } else {
        String::new()
    };

    for (key, accum) in &accum_map {
        let is_boundary = if !prev_composite.is_empty() {
            *key == prev_composite
        } else {
            accum.request_id == prev_request_id && !prev_request_id.is_empty()
        };

        let mut out = accum.output_tokens;
        if is_boundary {
            // This request was partially counted in the previous sync
            if out > prev_output_tokens {
                out -= prev_output_tokens;
            } else {
                out = 0;
            }
            // Don't re-count input tokens for boundary request
        } else {
            session.total_input_tokens += accum.input_tokens;
            session.total_cache_read_tokens += accum.cache_read_tokens;
            session.total_cache_creation_tokens += accum.cache_creation_tokens;
        }
        session.total_output_tokens += out;
        session.total_lines_added += accum.lines_added;
        session.total_lines_removed += accum.lines_removed;
        crate::aggregation::merge_extension_lines(&mut session.total_lines_by_extension, &accum.lines_by_extension);

        session.requests.push(RequestUsage {
            request_id: accum.request_id.clone(),
            message_id: accum.message_id.clone(),
            timestamp: accum.timestamp.clone(),
            model: accum.model.clone(),
            input_tokens: if is_boundary { 0 } else { accum.input_tokens },
            output_tokens: out,
            cache_read_tokens: if is_boundary { 0 } else { accum.cache_read_tokens },
            cache_creation_tokens: if is_boundary { 0 } else { accum.cache_creation_tokens },
            is_subagent: false,
            prompt_index: accum.prompt_index,
            tools: accum.tools.clone(),
            lines_added: accum.lines_added,
            lines_removed: accum.lines_removed,
            lines_by_extension: accum.lines_by_extension.clone(),
        });
    }

    let last_out = accum_map
        .values()
        .find(|a| a.request_id == last_request_id && a.message_id == last_message_id)
        .map(|a| a.output_tokens)
        .unwrap_or(0);

    Some((session, current_offset, last_request_id, last_message_id, last_out))
}

// ---- Subagent merging ----

pub fn merge_subagent_sessions(parent: &mut Session, subagent: Session) {
    // Build prompt timestamp ranges for attribution
    let mut prompt_ranges: Vec<(i32, String, String)> = Vec::new();
    {
        let mut prompt_timestamps: Vec<(i32, String)> = parent.prompts.iter()
            .map(|p| (p.prompt_index, p.timestamp.clone()))
            .collect();
        // Also check requests for prompt timestamps (prompts vec only has entries with tools)
        for req in &parent.requests {
            if !prompt_timestamps.iter().any(|(idx, _)| *idx == req.prompt_index) {
                prompt_timestamps.push((req.prompt_index, req.timestamp.clone()));
            }
        }
        prompt_timestamps.sort_by(|a, b| a.0.cmp(&b.0));
        for i in 0..prompt_timestamps.len() {
            let (idx, start) = &prompt_timestamps[i];
            let end = if i + 1 < prompt_timestamps.len() {
                prompt_timestamps[i + 1].1.clone()
            } else {
                "9999".to_string()
            };
            prompt_ranges.push((*idx, start.clone(), end));
        }
    }

    let mut prompts_with_subagent: HashSet<i32> = HashSet::new();

    for mut req in subagent.requests {
        req.is_subagent = true;
        // Assign prompt_index based on timestamp range
        if !prompt_ranges.is_empty() {
            for (idx, start, end) in &prompt_ranges {
                if req.timestamp >= *start && req.timestamp < *end {
                    req.prompt_index = *idx;
                    break;
                }
            }
            // If no range matched (e.g. after last prompt), assign to last prompt
            if !req.timestamp.is_empty() && req.prompt_index == 0 && !prompt_ranges.is_empty() {
                if let Some((last_idx, last_start, _)) = prompt_ranges.last() {
                    if req.timestamp >= *last_start {
                        req.prompt_index = *last_idx;
                    }
                }
            }
        }
        prompts_with_subagent.insert(req.prompt_index);
        parent.total_input_tokens += req.input_tokens;
        parent.total_output_tokens += req.output_tokens;
        parent.total_cache_read_tokens += req.cache_read_tokens;
        parent.total_cache_creation_tokens += req.cache_creation_tokens;
        parent.total_lines_added += req.lines_added;
        parent.total_lines_removed += req.lines_removed;
        crate::aggregation::merge_extension_lines(&mut parent.total_lines_by_extension, &req.lines_by_extension);
        parent.requests.push(req);
    }

    // Increment subagent_count for each prompt that received requests from this subagent
    for prompt in &mut parent.prompts {
        if prompts_with_subagent.contains(&prompt.prompt_index) {
            prompt.subagent_count += 1;
        }
    }
}
