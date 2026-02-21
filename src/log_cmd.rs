use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use chrono::Utc;
use serde_json::{json, Map, Value};
use crate::hash::hash_path;
use crate::paths::metrics_path;
use crate::sync::{cmd_sync, cmd_sync_transcripts, SYNC_BUFFER_THRESHOLD, SYNC_EVENTS};
use crate::transcripts::{read_cursors, write_cursors, derive_transcript_path};

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

pub fn cmd_log(dir: &Path) -> i32 {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() || input.is_empty() {
        return 0;
    }

    let mut evt: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => {
            let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let line = json!({"logged_at": ts, "parse_error": true});
            let path = metrics_path(dir);
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

    let event_name = obj
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

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

    if let Some(Value::Object(ti)) = obj.get_mut("tool_input") {
        strip_field_bytes(ti, "content");
        strip_field_bytes(ti, "old_string");
        strip_field_bytes(ti, "new_string");
        strip_field_bytes(ti, "new_source");
        strip_command(ti);
        strip_long_string(ti, "prompt", 200, 0);
        strip_long_string(ti, "description", 300, 100);
    }

    if let Some(resp) = obj.remove("tool_response") {
        let bytes = serde_json::to_string(&resp).map(|s| s.len()).unwrap_or(0);
        obj.insert("tool_response_bytes".to_string(), json!(bytes));
    }

    obj.remove("transcript_path");

    if let Some(Value::String(cwd)) = obj.remove("cwd") {
        obj.insert("path_hash".to_string(), json!(hash_path(&cwd)));
        let project_name = cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or("unknown");
        obj.insert("project".to_string(), json!(project_name));
    }

    if event_name == "UserPromptSubmit" {
        if let Some(Value::String(prompt)) = obj.remove("prompt") {
            obj.insert("prompt_bytes".to_string(), json!(prompt.len()));
        }
    }

    let path = metrics_path(dir);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", serde_json::to_string(&evt).unwrap_or_default());
    }

    let is_boundary = SYNC_EVENTS.contains(&event_name.as_str());
    let line_count = fs::read_to_string(&path)
        .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    if is_boundary || line_count >= SYNC_BUFFER_THRESHOLD {
        cmd_sync(dir);
    }

    0
}

pub fn cmd_log_transcripts(dir: &Path) -> i32 {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() || input.is_empty() {
        return 0;
    }

    let evt: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    let session_id = evt.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let cwd = evt.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
    let event_name = evt
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if session_id.is_empty() || cwd.is_empty() {
        return 0;
    }

    let transcript_path = evt
        .get("transcript_path")
        .and_then(|v| v.as_str())
        .map(|s| std::path::PathBuf::from(s))
        .unwrap_or_else(|| derive_transcript_path(cwd, session_id));

    let transcript_key = transcript_path.to_string_lossy().to_string();

    let path_hash_val = hash_path(cwd);
    let project_name = cwd
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("unknown");

    let mut cursors = read_cursors(dir);
    if !cursors.contains_key(&transcript_key) {
        cursors.insert(
            transcript_key,
            json!({
                "byte_offset": 0,
                "session_id": session_id,
                "project": project_name,
                "path_hash": path_hash_val,
                "last_request_id": "",
                "last_output_tokens": 0
            }),
        );
        write_cursors(dir, &cursors);
    }

    if SYNC_EVENTS.contains(&event_name) {
        cmd_sync_transcripts(dir);
    }

    0
}
