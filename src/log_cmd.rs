use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use chrono::Utc;
use serde_json::{json, Value};
use crate::hash::hash_path;
use crate::paths::{pending_path, sync_log};
use crate::sync::{cmd_sync_single_transcript, dump_transcript_debug, SYNC_EVENTS};
use crate::transcripts::{read_cursors, write_cursors};

// ---- Subagent pending state ----

fn read_pending(dir: &Path) -> HashMap<String, HashMap<String, String>> {
    let data = match fs::read_to_string(pending_path(dir)) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    let val: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let obj = match val.as_object() {
        Some(o) => o,
        None => return HashMap::new(),
    };
    obj.iter()
        .filter_map(|(k, v)| {
            let inner = v.as_object()?;
            let map: HashMap<String, String> = inner
                .iter()
                .filter_map(|(ik, iv)| Some((ik.clone(), iv.as_str()?.to_string())))
                .collect();
            Some((k.clone(), map))
        })
        .collect()
}

fn write_pending(dir: &Path, state: &HashMap<String, HashMap<String, String>>) {
    let val: Value = state
        .iter()
        .map(|(k, inner)| {
            let inner_val: Value = inner
                .iter()
                .map(|(ik, iv)| (ik.clone(), Value::String(iv.clone())))
                .collect::<serde_json::Map<String, Value>>()
                .into();
            (k.clone(), inner_val)
        })
        .collect::<serde_json::Map<String, Value>>()
        .into();
    let tmp = pending_path(dir).with_extension("json.tmp");
    let target = pending_path(dir);
    if let Ok(data) = serde_json::to_string_pretty(&val) {
        if fs::write(&tmp, &data).is_ok() {
            let _ = fs::rename(&tmp, &target);
        }
    }
}

// ---- Hook handler ----

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
                    "last_output_tokens": 0,
                    "last_prompt_count": 0
                }));
                write_cursors(dir, &cursors);
            }
        }
    }

    // Auto-update on SessionStart + clear stale subagent state
    if event_name == "SessionStart" {
        crate::update::auto_update(dir);
        if let Some(tp) = transcript_path {
            let mut pending = read_pending(dir);
            if pending.remove(tp).is_some() {
                write_pending(dir, &pending);
                sync_log(dir, "[hook] Cleared stale subagent state on SessionStart");
            }
        }
    }

    // Track subagent lifecycle
    let agent_id = obj.get("agent_id").and_then(|v| v.as_str());

    if event_name == "SubagentStart" {
        if let (Some(tp), Some(aid)) = (transcript_path, agent_id) {
            let mut pending = read_pending(dir);
            let entry = pending.entry(tp.to_string()).or_default();
            entry.insert(aid.to_string(), Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
            write_pending(dir, &pending);
            let session_short = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("?")
                .get(..12).unwrap_or("?");
            sync_log(dir, &format!("[hook] Subagent started: {} session={}", aid, session_short));
        }
    }

    if event_name == "SubagentStop" {
        if let (Some(tp), Some(aid)) = (transcript_path, agent_id) {
            let mut pending = read_pending(dir);
            if let Some(entry) = pending.get_mut(tp) {
                if entry.remove(aid).is_some() {
                    let remaining = entry.len();
                    let all_clear = entry.is_empty();
                    if all_clear {
                        pending.remove(tp);
                    }
                    write_pending(dir, &pending);
                    let session_short = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("?")
                        .get(..12).unwrap_or("?");
                    sync_log(dir, &format!(
                        "[hook] Subagent done: {} session={} remaining={}",
                        aid, session_short, remaining
                    ));
                    // All subagents complete - trigger the deferred sync now
                    if all_clear {
                        let auto_sync = crate::config::config_get_bool_default(dir, "autoSync", true);
                        if auto_sync {
                            sync_log(dir, "[hook] All subagents done, triggering deferred sync");
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            cmd_sync_single_transcript(dir, tp, "SubagentStop");
                        }
                    }
                } else {
                    sync_log(dir, &format!("[hook] Subagent orphan stop: {} (ignored)", aid));
                }
            } else {
                sync_log(dir, &format!("[hook] Subagent orphan stop: {} (no transcript entry)", aid));
            }
        }
    }

    let is_boundary = SYNC_EVENTS.contains(&event_name);

    let tool_name = obj.get("tool_name").and_then(|v| v.as_str()).unwrap_or("-");
    let session_short = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("?")
        .get(..12).unwrap_or("?");

    sync_log(dir, &format!(
        "[hook] {} tool={} session={} boundary={}",
        event_name, tool_name, session_short, is_boundary
    ));

    if is_boundary {
        // Wait for transcript writes to flush before parsing.
        // Stop fires before the final assistant message is fully written (~500ms race).
        std::thread::sleep(std::time::Duration::from_secs(2));

        let auto_sync = crate::config::config_get_bool_default(dir, "autoSync", true);
        if auto_sync {
            if let Some(tp) = transcript_path {
                let should_sync = if event_name == "SessionEnd" {
                    // Always sync on SessionEnd (session closing)
                    true
                } else {
                    check_subagents_clear(dir, tp)
                };

                if should_sync {
                    cmd_sync_single_transcript(dir, tp, event_name);
                }
            } else {
                sync_log(dir, "[hook] No transcript_path, skipping sync");
            }
        }
        if crate::config::config_get_bool(dir, "debugMode") {
            dump_transcript_debug(dir);
        }
    }

    0
}

/// Returns true if no subagents are pending for this transcript (safe to sync).
/// Applies a 10-minute timeout to stuck subagents.
fn check_subagents_clear(dir: &Path, transcript_path: &str) -> bool {
    let mut pending = read_pending(dir);
    let entry = match pending.get_mut(transcript_path) {
        Some(e) if !e.is_empty() => e,
        _ => return true, // no pending subagents
    };

    // Timeout: remove agents pending > 10 minutes
    let cutoff = (Utc::now() - chrono::Duration::minutes(10))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let before = entry.len();
    entry.retain(|agent_id, started_at| {
        if *started_at < cutoff {
            sync_log(dir, &format!("[hook] Subagent timeout (>10min): {}", agent_id));
            false
        } else {
            true
        }
    });

    if entry.len() != before {
        if entry.is_empty() {
            pending.remove(transcript_path);
        }
        write_pending(dir, &pending);
    }

    let still_pending = pending.get(transcript_path).map(|m| !m.is_empty()).unwrap_or(false);
    if still_pending {
        let count = pending.get(transcript_path).map(|m| m.len()).unwrap_or(0);
        sync_log(dir, &format!("[hook] {} subagent(s) pending, deferring sync", count));
    }

    !still_pending
}
