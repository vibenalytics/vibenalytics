use std::io::{self, Read};
use std::path::Path;
use serde_json::{json, Value};
use crate::hash::hash_path;
use crate::sync::{cmd_sync_transcripts, dump_transcript_debug, SYNC_EVENTS};
use crate::transcripts::{read_cursors, write_cursors};

pub fn cmd_log(dir: &Path) -> i32 {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() || input.is_empty() {
        return 0;
    }

    let evt: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    let obj = match evt.as_object() {
        Some(o) => o,
        None => return 0,
    };

    let event_name = obj
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let cwd = obj.get("cwd").and_then(|v| v.as_str()).unwrap_or("");

    // Block disabled projects (except SessionStart for project registration)
    if !cwd.is_empty() && event_name != "SessionStart" {
        let ph = hash_path(cwd);
        if crate::projects::is_project_enabled(dir, &ph) == Some(false) {
            return 0;
        }
    }

    // Register transcript cursor if not already known
    let transcript_path = obj.get("transcript_path").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    if let Some(tp) = transcript_path {
        let sid = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
        if !sid.is_empty() {
            let mut cursors = read_cursors(dir);
            if !cursors.contains_key(tp) {
                let ph = if !cwd.is_empty() { hash_path(cwd) } else { String::new() };
                let proj = cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or("unknown");
                cursors.insert(tp.to_string(), json!({
                    "byte_offset": 0,
                    "session_id": sid,
                    "project": proj,
                    "path_hash": ph,
                    "last_request_id": "",
                    "last_output_tokens": 0
                }));
                write_cursors(dir, &cursors);
            }
        }
    }

    let is_boundary = SYNC_EVENTS.contains(&event_name);

    let tool_name = obj.get("tool_name").and_then(|v| v.as_str()).unwrap_or("-");
    let session_short = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("?")
        .get(..12).unwrap_or("?");

    crate::paths::sync_log(dir, &format!(
        "[hook] {} tool={} session={} boundary={}",
        event_name, tool_name, session_short, is_boundary
    ));

    if is_boundary {
        let auto_sync = crate::config::config_get_bool_default(dir, "autoSync", true);
        if auto_sync {
            cmd_sync_transcripts(dir);
        }
        if crate::config::config_get_bool(dir, "debugMode") {
            dump_transcript_debug(dir);
        }
    }

    0
}
