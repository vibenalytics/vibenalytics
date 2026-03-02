use std::collections::HashMap;
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
    pub tools: HashMap<String, u32>,
    pub lines_added: u64,
    pub lines_removed: u64,
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
    pub skills: Vec<String>,
    pub subagent_count: u32,
    pub compaction_trigger: String,
    pub compaction_pre_tokens: u64,
    pub context_tokens: u64,
}

const BUILTIN_COMMANDS: &[&str] = &[
    "/compact", "/clear", "/exit", "/help", "/login", "/logout",
    "/config", "/model", "/resume", "/copy", "/debug", "/mcp",
    "/plugin", "/context", "/init", "/rate-limit-options",
    "/remote-env", "/extra-usage", "/passes",
];

/// Returns (msg_type, command, skills).
/// - Built-in commands: ("command", "/compact", [])
/// - Skill invocations: ("prompt", "", ["/frontend-design"])
/// - Mixed prompt with skill: ("prompt", "", ["/frontend-design"])
/// - Regular prompt: ("prompt", "", [])
pub fn classify_prompt(text: &str) -> (String, String, Vec<String>) {
    let trimmed = text.trim_start();

    let cmd = extract_command_name(trimmed);

    if cmd.is_empty() {
        return ("prompt".to_string(), String::new(), Vec::new());
    }

    let is_pure_command = trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<command-message>");

    if is_pure_command && BUILTIN_COMMANDS.contains(&cmd.as_str()) {
        ("command".to_string(), cmd, Vec::new())
    } else {
        // Skill invocation (standalone or inline) - it's a prompt with skill metadata
        ("prompt".to_string(), String::new(), vec![cmd])
    }
}

fn extract_command_name(text: &str) -> String {
    if let Some(start) = text.find("<command-name>") {
        let after = &text[start + "<command-name>".len()..];
        if let Some(end) = after.find("</command-name>") {
            return after[..end].to_string();
        }
    }
    String::new()
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
    pub total_lines_added: u64,
    pub total_lines_removed: u64,
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
            total_lines_added: 0,
            total_lines_removed: 0,
        }
    }
}

pub fn parse_iso_timestamp(ts: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ"))
        .ok()
        .map(|dt| dt.and_utc().timestamp())
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
            skills: Vec::new(),
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
            skills: Vec::new(),
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
        for skill in &prompt.skills {
            if !p.skills.contains(skill) {
                p.skills.push(skill.clone());
            }
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
            obj["total_lines_added"] = json!(s.total_lines_added);
            obj["total_lines_removed"] = json!(s.total_lines_removed);
            if !s.requests.is_empty() {
                // Group requests by prompt_index for nesting inside prompts
                let mut requests_by_prompt: HashMap<i32, Vec<&RequestUsage>> = HashMap::new();
                for r in &s.requests {
                    requests_by_prompt.entry(r.prompt_index).or_default().push(r);
                }

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
                        if !p.skills.is_empty() {
                            pobj["skills"] = json!(p.skills);
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
                        if let Some(reqs) = requests_by_prompt.get(&p.prompt_index) {
                            pobj["requests"] = json!(reqs.iter().map(|r| {
                                let mut robj = json!({
                                    "request_id": r.request_id,
                                    "message_id": r.message_id,
                                    "timestamp": r.timestamp,
                                    "model": r.model,
                                    "input_tokens": r.input_tokens,
                                    "output_tokens": r.output_tokens,
                                    "cache_read_tokens": r.cache_read_tokens,
                                    "cache_creation_tokens": r.cache_creation_tokens,
                                    "is_subagent": r.is_subagent,
                                    "tools": r.tools,
                                });
                                robj["lines_added"] = json!(r.lines_added);
                                robj["lines_removed"] = json!(r.lines_removed);
                                robj
                            }).collect::<Vec<_>>());
                        }
                        pobj
                    }).collect::<Vec<_>>());
                }
            }
            obj
        })
        .collect();
    json!({ "sessions": arr })
}
