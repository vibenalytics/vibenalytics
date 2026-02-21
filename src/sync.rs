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
    match fs::metadata(&mp) {
        Ok(m) if m.len() > 0 => {}
        _ => {
            sync_log(dir, "No metrics to sync");
            return 0;
        }
    };

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

    let sessions = aggregate_file(&mp);
    let sessions = crate::projects::filter_sessions_by_enabled(dir, sessions);

    if sessions.is_empty() {
        sync_log(dir, "No sessions to sync");
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let payload = build_payload(&sessions);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let n = sessions.len();

    sync_log(dir, &format!("Syncing {n} sessions"));

    let url = format!("{api_base}/sync");
    let result = http_post(&url, &payload_str, Some(&api_key));

    let rc = match result {
        Ok((200, resp)) => {
            sync_log(dir, &format!("Sync OK: {resp}"));
            let ts = Utc::now().format("%Y%m%d_%H%M%S");
            let archive = dir.join(format!("metrics.synced.{ts}.jsonl"));
            let _ = fs::rename(&mp, &archive);
            let _ = fs::write(&mp, "");
            sync_log(
                dir,
                &format!(
                    "Flushed metrics.jsonl, archived to {}",
                    archive.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
            0
        }
        Ok((code, resp)) => {
            sync_log(dir, &format!("Sync FAILED (HTTP {code}): {resp}"));
            1
        }
        Err(e) => {
            sync_log(dir, &format!("Sync FAILED: {e}"));
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

    let sessions = crate::projects::filter_sessions_by_enabled(dir, sessions);

    if sessions.is_empty() {
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let payload = build_payload(&sessions);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let n = sessions.len();

    sync_log(dir, &format!("[transcripts] Syncing {n} sessions"));

    let url = format!("{api_base}/sync");
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
