use std::collections::HashMap;
use std::fs;
use std::path::Path;
use serde_json::{json, Value};

pub struct RequestUsage {
    pub request_id: String,
    pub message_id: String,
    pub timestamp: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub is_subagent: bool,
    pub prompt_index: i32,
}

pub struct PromptUsage {
    pub prompt_index: i32,
    pub timestamp: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub request_count: u32,
    pub tools: HashMap<String, u32>,
    pub model: String,
    pub prompt_text: String,
    pub msg_type: String,
    pub command: String,
    pub subagent_count: u32,
    pub compaction_trigger: String,
    pub compaction_pre_tokens: u64,
    pub context_tokens: u64,
}

pub fn classify_prompt(text: &str) -> (String, String) {
    let trimmed = text.trim_start();
    if trimmed.starts_with("<command-name>") {
        let cmd = trimmed
            .strip_prefix("<command-name>")
            .and_then(|s| s.split("</command-name>").next())
            .unwrap_or("")
            .to_string();
        ("command".to_string(), cmd)
    } else {
        ("prompt".to_string(), String::new())
    }
}

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
    pub project_path: String,
    pub started_at: String,
    pub ended_at: String,
    pub permission_mode: String,
    pub events: HashMap<String, u32>,
    pub tools: HashMap<String, u32>,
    pub prompt_count: u32,
    pub message_count: u32,
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
    pub claude_version: String,
    pub requests: Vec<RequestUsage>,
    pub prompts: Vec<PromptUsage>,
}

impl crate::projects::HasProjectHash for Session {
    fn path_hash(&self) -> &str { &self.path_hash }
    fn project_name(&self) -> &str { &self.project }
    fn project_path(&self) -> &str { &self.project_path }
}

impl Session {
    pub fn new(id: &str) -> Self {
        Session {
            session_id: id.to_string(),
            project: "unknown".to_string(),
            path_hash: String::new(),
            project_path: String::new(),
            started_at: String::new(),
            ended_at: String::new(),
            permission_mode: String::new(),
            events: HashMap::new(),
            tools: HashMap::new(),
            prompt_count: 0,
            message_count: 0,
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
            claude_version: String::new(),
            requests: Vec::new(),
            prompts: Vec::new(),
        }
    }
}

pub fn parse_iso_timestamp(ts: &str) -> Option<i64> {
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

        if s.project_path.is_empty() {
            if let Some(cwd) = evt.get("_cwd").and_then(|v| v.as_str()) {
                s.project_path = cwd.to_string();
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

        if let Some(pm) = evt.get("permission_mode").and_then(|v| v.as_str()) {
            s.permission_mode = pm.to_string();
        }
    }

    sessions
}

/// Build PromptUsage list by grouping requests by prompt_index.
pub fn build_prompts(session: &Session) -> Vec<PromptUsage> {
    let mut map: HashMap<i32, PromptUsage> = HashMap::new();

    for req in &session.requests {
        let p = map.entry(req.prompt_index).or_insert_with(|| PromptUsage {
            prompt_index: req.prompt_index,
            timestamp: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_count: 0,
            tools: HashMap::new(),
            model: String::new(),
            prompt_text: String::new(),
            msg_type: String::new(),
            command: String::new(),
            subagent_count: 0,
            compaction_trigger: String::new(),
            compaction_pre_tokens: 0,
            context_tokens: 0,
        });
        // Context size from the first request of this prompt (full conversation context)
        if p.context_tokens == 0 {
            p.context_tokens = req.input_tokens + req.cache_read_tokens + req.cache_creation_tokens;
        }
        p.input_tokens += req.input_tokens;
        p.output_tokens += req.output_tokens;
        p.cache_read_tokens += req.cache_read_tokens;
        p.cache_creation_tokens += req.cache_creation_tokens;
        p.request_count += 1;
        if p.model.is_empty() && !req.model.is_empty() {
            p.model = req.model.clone();
        }
        if p.timestamp.is_empty() || (!req.timestamp.is_empty() && req.timestamp < p.timestamp) {
            p.timestamp = req.timestamp.clone();
        }
    }

    // Merge per-prompt tool counts, text, and metadata from session.prompts (set during parsing)
    for prompt in &session.prompts {
        let p = map.entry(prompt.prompt_index).or_insert_with(|| PromptUsage {
            prompt_index: prompt.prompt_index,
            timestamp: prompt.timestamp.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_count: 0,
            tools: HashMap::new(),
            model: String::new(),
            prompt_text: String::new(),
            msg_type: String::new(),
            command: String::new(),
            subagent_count: 0,
            compaction_trigger: String::new(),
            compaction_pre_tokens: 0,
            context_tokens: 0,
        });
        for (tool, count) in &prompt.tools {
            *p.tools.entry(tool.clone()).or_insert(0) += count;
        }
        if !prompt.prompt_text.is_empty() {
            p.prompt_text = prompt.prompt_text.clone();
        }
        if !prompt.msg_type.is_empty() {
            p.msg_type = prompt.msg_type.clone();
        }
        if !prompt.command.is_empty() {
            p.command = prompt.command.clone();
        }
        if prompt.subagent_count > 0 {
            p.subagent_count += prompt.subagent_count;
        }
        if !prompt.compaction_trigger.is_empty() {
            p.compaction_trigger = prompt.compaction_trigger.clone();
            p.compaction_pre_tokens = prompt.compaction_pre_tokens;
        }
        if p.timestamp.is_empty() && !prompt.timestamp.is_empty() {
            p.timestamp = prompt.timestamp.clone();
        }
    }

    let mut result: Vec<PromptUsage> = map.into_values().collect();
    result.sort_by_key(|p| p.prompt_index);
    result
}

pub fn build_payload(sessions: &[Session]) -> Value {
    let arr: Vec<Value> = sessions
        .iter()
        .filter(|s| !s.requests.is_empty())
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
                "message_count": s.message_count,
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
            if !s.claude_version.is_empty() {
                obj["claude_version"] = json!(s.claude_version);
            }
            if !s.requests.is_empty() {
                let prompts = build_prompts(s);
                if !prompts.is_empty() {
                    obj["prompts"] = json!(prompts.iter().map(|p| {
                        let msg_type = if p.msg_type.is_empty() { "prompt" } else { &p.msg_type };
                        let mut pobj = json!({
                            "prompt_index": p.prompt_index,
                            "timestamp": p.timestamp,
                            "type": msg_type,
                            "input_tokens": p.input_tokens,
                            "output_tokens": p.output_tokens,
                            "cache_read_tokens": p.cache_read_tokens,
                            "cache_creation_tokens": p.cache_creation_tokens,
                            "context_tokens": p.context_tokens,
                            "request_count": p.request_count,
                        });
                        if !p.command.is_empty() {
                            pobj["command"] = json!(p.command);
                        }
                        if !p.tools.is_empty() {
                            pobj["tools"] = json!(p.tools);
                        }
                        if !p.model.is_empty() {
                            pobj["model"] = json!(p.model);
                        }
                        if !p.prompt_text.is_empty() {
                            pobj["prompt_text"] = json!(p.prompt_text);
                        }
                        if !p.compaction_trigger.is_empty() {
                            pobj["compaction_trigger"] = json!(p.compaction_trigger);
                            pobj["compaction_pre_tokens"] = json!(p.compaction_pre_tokens);
                        }
                        if p.subagent_count > 0 {
                            pobj["subagent_count"] = json!(p.subagent_count);
                        }
                        pobj
                    }).collect::<Vec<_>>());
                }
                obj["requests"] = json!(s.requests.iter().map(|r| json!({
                    "request_id": r.request_id,
                    "message_id": r.message_id,
                    "timestamp": r.timestamp,
                    "model": r.model,
                    "input_tokens": r.input_tokens,
                    "output_tokens": r.output_tokens,
                    "cache_read_tokens": r.cache_read_tokens,
                    "cache_creation_tokens": r.cache_creation_tokens,
                    "is_subagent": r.is_subagent,
                    "prompt_index": r.prompt_index,
                })).collect::<Vec<_>>());
            }
            if s.requests.iter().any(|r| r.is_subagent) {
                let sub_in: u64 = s.requests.iter().filter(|r| r.is_subagent).map(|r| r.input_tokens).sum();
                let sub_out: u64 = s.requests.iter().filter(|r| r.is_subagent).map(|r| r.output_tokens).sum();
                obj["subagent_input_tokens"] = json!(sub_in);
                obj["subagent_output_tokens"] = json!(sub_out);
            }
            obj
        })
        .collect();
    json!({ "sessions": arr })
}
