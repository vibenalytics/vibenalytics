use std::fs;
use std::path::Path;
use std::time::SystemTime;
use chrono::Utc;
use serde_json::json;
use crate::config::{config_get, DEFAULT_API_BASE};
use crate::paths::{metrics_path, cursors_path, sync_log};
use crate::http::http_post;
use crate::aggregation::{aggregate_file, build_payload, Session};
use crate::transcripts::{read_cursors, write_cursors, parse_transcript_from_offset};

pub const SYNC_BUFFER_THRESHOLD: usize = 10;
pub const SYNC_EVENTS: &[&str] = &["SessionStart", "SessionEnd", "Stop", "UserPromptSubmit"];

pub fn cmd_sync(dir: &Path) -> i32 {
    let api_key = match config_get(dir, "apiKey") {
        Some(key) => key,
        None => {
            sync_log(dir, "No API key configured — skipping sync");
            return 0;
        }
    };

    let api_base = DEFAULT_API_BASE;

    let mp = metrics_path(dir);
    let lock = mp.with_extension("jsonl.lock");
    if lock.exists() {
        if let Ok(lm) = fs::metadata(&lock) {
            if let Ok(age) = lm
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .elapsed()
            {
                if age.as_secs() < 60 {
                    sync_log(dir, "Lock file exists, skipping sync");
                    return 0;
                }
            }
        }
        let _ = fs::remove_file(&lock);
        sync_log(dir, "Removed stale lock");
    }

    let _ = fs::write(&lock, format!("{}", std::process::id()));

    // Recover orphaned staging file from a previous crash
    let staging = mp.with_extension("staging.jsonl");
    if staging.exists() {
        if let Ok(staged_data) = fs::read_to_string(&staging) {
            if !staged_data.trim().is_empty() {
                let existing = fs::read_to_string(&mp).unwrap_or_default();
                let _ = fs::write(&mp, format!("{staged_data}{existing}"));
                sync_log(dir, "Recovered orphaned staging file");
            }
        }
        let _ = fs::remove_file(&staging);
    }

    // Re-check: metrics file might not exist or be empty
    match fs::metadata(&mp) {
        Ok(m) if m.len() > 0 => {}
        _ => {
            let _ = fs::remove_file(&lock);
            return 0;
        }
    }

    // Atomically swap: rename metrics.jsonl → staging so new events
    // go to a fresh file while we sync the old batch.
    let file_size = fs::metadata(&mp).map(|m| m.len()).unwrap_or(0);
    sync_log(dir, &format!("Staging metrics.jsonl ({file_size} bytes)"));
    if fs::rename(&mp, &staging).is_err() {
        sync_log(dir, "Failed to stage metrics file");
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let sessions = aggregate_file(&staging);
    let pre_filter = sessions.len();
    let sessions = crate::projects::filter_sessions_by_enabled(dir, sessions);

    if sessions.is_empty() {
        sync_log(dir, &format!("No enabled sessions to sync ({pre_filter} filtered out)"));
        let _ = fs::rename(&staging, &mp);
        let _ = fs::remove_file(&lock);
        return 0;
    }

    // Log detailed session info
    for s in &sessions {
        let tool_count: u32 = s.tools.values().sum();
        sync_log(dir, &format!(
            "  session={} project={} prompts={} tools={} events={} input_bytes={} response_bytes={}",
            &s.session_id[..s.session_id.len().min(12)],
            s.project, s.prompt_count, tool_count,
            s.events.values().sum::<u32>(),
            s.total_input_bytes, s.total_response_bytes
        ));
    }

    let payload = build_payload(&sessions);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let n = sessions.len();

    let url = format!("{api_base}/sync");
    sync_log(dir, &format!("POST {url} — {n} sessions, {} bytes payload", payload_str.len()));

    let result = http_post(&url, &payload_str, Some(&api_key));

    let rc = match result {
        Ok((200, resp)) => {
            sync_log(dir, &format!("Sync OK: {resp}"));
            let ts = Utc::now().format("%Y%m%d_%H%M%S");
            let archive = dir.join(format!("metrics.synced.{ts}.jsonl"));
            let _ = fs::rename(&staging, &archive);
            sync_log(
                dir,
                &format!(
                    "Archived to {}",
                    archive.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
            0
        }
        Ok((code, resp)) => {
            sync_log(dir, &format!("Sync FAILED (HTTP {code}): {resp}"));
            // Prepend staging data back so it's retried next sync
            if let Ok(staged_data) = fs::read_to_string(&staging) {
                let existing = fs::read_to_string(&mp).unwrap_or_default();
                let _ = fs::write(&mp, format!("{staged_data}{existing}"));
            }
            let _ = fs::remove_file(&staging);
            1
        }
        Err(e) => {
            sync_log(dir, &format!("Sync FAILED: {e}"));
            // Prepend staging data back so it's retried next sync
            if let Ok(staged_data) = fs::read_to_string(&staging) {
                let existing = fs::read_to_string(&mp).unwrap_or_default();
                let _ = fs::write(&mp, format!("{staged_data}{existing}"));
            }
            let _ = fs::remove_file(&staging);
            1
        }
    };

    let _ = fs::remove_file(&lock);
    rc
}

pub fn cmd_sync_transcripts(dir: &Path) -> i32 {
    let api_key = match config_get(dir, "apiKey") {
        Some(key) => key,
        None => {
            sync_log(dir, "[transcripts] No API key configured — skipping sync");
            return 0;
        }
    };

    let api_base = DEFAULT_API_BASE;

    let mut cursors = read_cursors(dir);
    if cursors.is_empty() {
        return 0;
    }

    let lock = cursors_path(dir).with_extension("json.lock");
    if lock.exists() {
        if let Ok(lm) = fs::metadata(&lock) {
            if let Ok(age) = lm
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .elapsed()
            {
                if age.as_secs() < 60 {
                    return 0;
                }
            }
        }
        let _ = fs::remove_file(&lock);
    }
    let _ = fs::write(&lock, format!("{}", std::process::id()));

    let mut sessions: Vec<Session> = Vec::new();
    let mut updated_cursors: Vec<(String, serde_json::Value)> = Vec::new();

    for (transcript_key, cursor_val) in &cursors {
        let transcript_path = Path::new(transcript_key.as_str());
        if !transcript_path.exists() {
            continue;
        }

        let byte_offset = cursor_val.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let prev_request_id = cursor_val.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_output_tokens = cursor_val.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let fallback_project = cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
        let fallback_path_hash = cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or("");

        let file_size = fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(0);
        if file_size <= byte_offset {
            continue;
        }

        if let Some((session, new_offset, last_rid, last_out)) = parse_transcript_from_offset(
            transcript_path,
            byte_offset,
            prev_request_id,
            prev_output_tokens,
            fallback_project,
            fallback_path_hash,
        ) {
            sessions.push(session);
            updated_cursors.push((
                transcript_key.clone(),
                json!({
                    "byte_offset": new_offset,
                    "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "project": cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown"),
                    "path_hash": cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or(""),
                    "last_request_id": last_rid,
                    "last_output_tokens": last_out
                }),
            ));
        }
    }

    let pre_filter = sessions.len();
    let sessions = crate::projects::filter_sessions_by_enabled(dir, sessions);

    if sessions.is_empty() {
        if pre_filter > 0 {
            sync_log(dir, &format!("[transcripts] No enabled sessions ({pre_filter} filtered out)"));
        }
        let _ = fs::remove_file(&lock);
        return 0;
    }

    for s in &sessions {
        let tool_count: u32 = s.tools.values().sum();
        sync_log(dir, &format!(
            "[transcripts]   session={} project={} prompts={} tools={} tokens_in={} tokens_out={} model={}",
            &s.session_id[..s.session_id.len().min(12)],
            s.project, s.prompt_count, tool_count,
            s.total_input_tokens, s.total_output_tokens, s.model
        ));
    }

    let payload = build_payload(&sessions);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let n = sessions.len();

    let url = format!("{api_base}/sync");
    sync_log(dir, &format!("[transcripts] POST {url} — {n} sessions, {} bytes payload", payload_str.len()));

    let result = http_post(&url, &payload_str, Some(&api_key));

    let rc = match result {
        Ok((200, resp)) => {
            sync_log(dir, &format!("[transcripts] Sync OK: {resp}"));
            for (key, val) in updated_cursors {
                cursors.insert(key, val);
            }
            write_cursors(dir, &cursors);
            0
        }
        Ok((code, resp)) => {
            sync_log(dir, &format!("[transcripts] Sync FAILED (HTTP {code}): {resp}"));
            1
        }
        Err(e) => {
            sync_log(dir, &format!("[transcripts] Sync FAILED: {e}"));
            1
        }
    };

    let _ = fs::remove_file(&lock);
    rc
}
