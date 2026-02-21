use std::collections::HashMap;
use std::fs;
use std::path::Path;
use serde_json::{json, Value};

pub struct ToolLatency {
    pub tool: String,
    pub total_ms: u64,
    pub count: u32,
    pub min_ms: u64,
    pub max_ms: u64,
}

pub struct PermissionStat {
    pub tool: String,
    pub domain: String,
    pub count: u32,
}

pub struct Session {
    pub session_id: String,
    pub project: String,
    pub path_hash: String,
    pub started_at: String,
    pub ended_at: String,
    pub permission_mode: String,
    pub events: HashMap<String, u32>,
    pub tools: HashMap<String, u32>,
    pub prompt_count: u32,
    pub message_count: u32,
    pub total_input_bytes: u64,
    pub total_response_bytes: u64,
    pub tool_latencies: Vec<ToolLatency>,
    pub permission_requests: Vec<PermissionStat>,
    pub tool_response_sizes: HashMap<String, (u64, u32)>,
    pub parallel_tool_batches: u32,
    pub hostname: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_turn_duration_ms: u64,
    pub turn_count: u32,
    pub model: String,
}

impl Session {
    pub fn new(id: &str) -> Self {
        Session {
            session_id: id.to_string(),
            project: "unknown".to_string(),
            path_hash: String::new(),
            started_at: String::new(),
            ended_at: String::new(),
            permission_mode: String::new(),
            events: HashMap::new(),
            tools: HashMap::new(),
            prompt_count: 0,
            message_count: 0,
            total_input_bytes: 0,
            total_response_bytes: 0,
            tool_latencies: Vec::new(),
            permission_requests: Vec::new(),
            tool_response_sizes: HashMap::new(),
            parallel_tool_batches: 0,
            hostname: String::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_turn_duration_ms: 0,
            turn_count: 0,
            model: String::new(),
        }
    }
}

pub fn parse_iso_timestamp(ts: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

/// Parse ISO 8601 timestamp with optional milliseconds
pub fn parse_iso_flex(ts: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ"))
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

pub fn aggregate_file(filepath: &Path) -> Vec<Session> {
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

        if let Some(ph) = evt.get("path_hash").and_then(|v| v.as_str()) {
            if s.path_hash.is_empty() {
                s.path_hash = ph.to_string();
            }
        }

        if let Some(proj) = evt.get("project").and_then(|v| v.as_str()) {
            if proj != "unknown" {
                s.project = proj.to_string();
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

pub fn build_payload(sessions: &[Session]) -> Value {
    let arr: Vec<Value> = sessions
        .iter()
        .map(|s| {
            let mut obj = json!({
                "session_id": s.session_id,
                "project_hash": s.path_hash,
                "project_name": s.project,
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
            if s.total_input_tokens > 0 || s.total_output_tokens > 0 {
                obj["total_input_tokens"] = json!(s.total_input_tokens);
                obj["total_output_tokens"] = json!(s.total_output_tokens);
                obj["total_cache_read_tokens"] = json!(s.total_cache_read_tokens);
                obj["total_cache_creation_tokens"] = json!(s.total_cache_creation_tokens);
            }
            if s.total_turn_duration_ms > 0 {
                obj["total_turn_duration_ms"] = json!(s.total_turn_duration_ms);
                obj["turn_count"] = json!(s.turn_count);
            }
            if !s.model.is_empty() {
                obj["model"] = json!(s.model);
            }
            obj
        })
        .collect();
    json!({ "sessions": arr })
}
