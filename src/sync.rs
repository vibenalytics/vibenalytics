use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;
use chrono::Utc;
use serde_json::json;
use crate::config::{config_get, config_get_bool, DEFAULT_API_BASE};
use crate::paths::{cursors_path, sync_log};
use crate::http::http_post;
use crate::aggregation::{build_payload, Session};
use crate::transcripts::{read_cursors, write_cursors, parse_transcript_from_offset, find_subagent_files, merge_subagent_sessions};

/// If debugMode is enabled, write the raw sync payload to `sync-debug/` as a timestamped JSON file.
fn dump_sync_payload(dir: &Path, label: &str, payload: &serde_json::Value) {
    if !config_get_bool(dir, "debugMode") {
        return;
    }
    let debug_dir = dir.join("sync-debug");
    let _ = fs::create_dir_all(&debug_dir);
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let debug_file = debug_dir.join(format!("{label}_{ts}.json"));
    if let Ok(pretty) = serde_json::to_string_pretty(payload) {
        if let Ok(mut f) = fs::File::create(&debug_file) {
            let _ = f.write_all(pretty.as_bytes());
        }
    }
    sync_log(dir, &format!(
        "[debug] Dumped {label} payload to {}",
        debug_file.file_name().unwrap_or_default().to_string_lossy()
    ));
}

pub const SYNC_EVENTS: &[&str] = &["Stop", "SessionEnd"];

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
        let prev_message_id = cursor_val.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_output_tokens = cursor_val.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let fallback_project = cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
        let fallback_path_hash = cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or("");

        let file_size = fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(0);
        if file_size == 0 || file_size == byte_offset {
            continue;
        }

        // If file shrunk (rotation/truncation), reset cursor to re-parse from start
        let (parse_offset, parse_prev_rid, parse_prev_mid, parse_prev_out) = if file_size < byte_offset {
            sync_log(dir, &format!(
                "[transcripts] File shrunk ({}B < {}B offset), resetting cursor",
                file_size, byte_offset
            ));
            (0u64, "", "", 0u64)
        } else {
            (byte_offset, prev_request_id, prev_message_id, prev_output_tokens)
        };

        if let Some((mut session, new_offset, last_rid, last_mid, last_out)) = parse_transcript_from_offset(
            transcript_path,
            parse_offset,
            parse_prev_rid,
            parse_prev_mid,
            parse_prev_out,
            fallback_project,
            fallback_path_hash,
            0,
        ) {
            // Discover and parse subagent files for this session
            let subagent_files = find_subagent_files(transcript_path);
            for sub_path in &subagent_files {
                let sub_key = sub_path.to_string_lossy().to_string();
                let (sub_offset, sub_prev_rid, sub_prev_mid, sub_prev_out) =
                    if let Some(sub_cursor) = cursors.get(&sub_key) {
                        (
                            sub_cursor.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0),
                            sub_cursor.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            sub_cursor.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            sub_cursor.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        )
                    } else {
                        (0u64, String::new(), String::new(), 0u64)
                    };

                let sub_file_size = fs::metadata(sub_path).map(|m| m.len()).unwrap_or(0);
                if sub_file_size == 0 || sub_file_size == sub_offset {
                    continue;
                }

                if let Some((sub_session, sub_new_offset, sub_last_rid, sub_last_mid, sub_last_out)) =
                    parse_transcript_from_offset(
                        sub_path,
                        sub_offset,
                        &sub_prev_rid,
                        &sub_prev_mid,
                        sub_prev_out,
                        fallback_project,
                        fallback_path_hash,
                        0,
                    )
                {
                    merge_subagent_sessions(&mut session, sub_session);
                    updated_cursors.push((
                        sub_key,
                        json!({
                            "byte_offset": sub_new_offset,
                            "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                            "project": fallback_project,
                            "path_hash": fallback_path_hash,
                            "last_request_id": sub_last_rid,
                            "last_message_id": sub_last_mid,
                            "last_output_tokens": sub_last_out
                        }),
                    ));
                }
            }

            sessions.push(session);
            updated_cursors.push((
                transcript_key.clone(),
                json!({
                    "byte_offset": new_offset,
                    "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "project": cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown"),
                    "path_hash": cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or(""),
                    "last_request_id": last_rid,
                    "last_message_id": last_mid,
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
    dump_sync_payload(dir, "transcripts", &payload);
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

/// Sync a single transcript file (triggered by hook boundary events).
/// Only syncs data from the specific transcript that triggered the event.
pub fn cmd_sync_single_transcript(dir: &Path, transcript_path_str: &str, event_name: &str) -> i32 {
    let api_key = match config_get(dir, "apiKey") {
        Some(key) => key,
        None => return 0,
    };

    let api_base = DEFAULT_API_BASE;

    let mut cursors = read_cursors(dir);
    let cursor_val = match cursors.get(transcript_path_str) {
        Some(c) => c.clone(),
        None => {
            sync_log(dir, &format!("[single] No cursor for {}, skipping", transcript_path_str));
            return 0;
        }
    };

    let transcript_path = std::path::Path::new(transcript_path_str);
    if !transcript_path.exists() {
        sync_log(dir, &format!("[single] Transcript file not found: {}", transcript_path_str));
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
                    sync_log(dir, "[single] Lock held, skipping");
                    return 0;
                }
            }
        }
        let _ = fs::remove_file(&lock);
    }
    let _ = fs::write(&lock, format!("{}", std::process::id()));

    let byte_offset = cursor_val.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0);
    let prev_request_id = cursor_val.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("");
    let prev_message_id = cursor_val.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("");
    let prev_output_tokens = cursor_val.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let fallback_project = cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
    let fallback_path_hash = cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or("");
    let last_prompt_count = cursor_val.get("last_prompt_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    let file_size = fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(0);
    if file_size == 0 || file_size == byte_offset {
        sync_log(dir, &format!("[single] No new data (file={}B, cursor={}B)", file_size, byte_offset));
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let (parse_offset, parse_prev_rid, parse_prev_mid, parse_prev_out) = if file_size < byte_offset {
        sync_log(dir, &format!(
            "[single] File shrunk ({}B < {}B offset), resetting cursor",
            file_size, byte_offset
        ));
        (0u64, "", "", 0u64)
    } else {
        (byte_offset, prev_request_id, prev_message_id, prev_output_tokens)
    };

    let prompt_offset = if file_size < byte_offset { 0 } else { last_prompt_count };

    let result = parse_transcript_from_offset(
        transcript_path,
        parse_offset,
        parse_prev_rid,
        parse_prev_mid,
        parse_prev_out,
        fallback_project,
        fallback_path_hash,
        prompt_offset,
    );

    let (mut session, new_offset, last_rid, last_mid, last_out) = match result {
        Some(r) => r,
        None => {
            sync_log(dir, "[single] Parse returned no data");
            let _ = fs::remove_file(&lock);
            return 0;
        }
    };

    // Discover and parse subagent files
    let subagent_files = find_subagent_files(transcript_path);
    let mut sub_updated: Vec<(String, serde_json::Value)> = Vec::new();
    for sub_path in &subagent_files {
        let sub_key = sub_path.to_string_lossy().to_string();
        let (sub_offset, sub_prev_rid, sub_prev_mid, sub_prev_out) =
            if let Some(sub_cursor) = cursors.get(&sub_key) {
                (
                    sub_cursor.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0),
                    sub_cursor.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    sub_cursor.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    sub_cursor.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                )
            } else {
                (0u64, String::new(), String::new(), 0u64)
            };

        let sub_file_size = fs::metadata(sub_path).map(|m| m.len()).unwrap_or(0);
        if sub_file_size == 0 || sub_file_size == sub_offset {
            continue;
        }

        if let Some((sub_session, sub_new_offset, sub_last_rid, sub_last_mid, sub_last_out)) =
            parse_transcript_from_offset(
                sub_path,
                sub_offset,
                &sub_prev_rid,
                &sub_prev_mid,
                sub_prev_out,
                fallback_project,
                fallback_path_hash,
                0,
            )
        {
            merge_subagent_sessions(&mut session, sub_session);
            sub_updated.push((
                sub_key,
                json!({
                    "byte_offset": sub_new_offset,
                    "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "project": fallback_project,
                    "path_hash": fallback_path_hash,
                    "last_request_id": sub_last_rid,
                    "last_message_id": sub_last_mid,
                    "last_output_tokens": sub_last_out
                }),
            ));
        }
    }

    // Check project is enabled
    if !session.path_hash.is_empty() {
        if crate::projects::is_project_enabled(dir, &session.path_hash) == Some(false) {
            let _ = fs::remove_file(&lock);
            return 0;
        }
    }

    // Skip empty syncs (no new prompts or tokens) - just advance cursor
    if session.prompt_count == 0
        && session.total_input_tokens == 0
        && session.total_output_tokens == 0
    {
        let sid_short = &session.session_id[..session.session_id.len().min(12)];
        sync_log(dir, &format!(
            "[single] {} No new data, advancing cursor event={}",
            sid_short, event_name
        ));
        cursors.insert(
            transcript_path_str.to_string(),
            json!({
                "byte_offset": new_offset,
                "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                "project": cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "path_hash": cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or(""),
                "last_request_id": last_rid,
                "last_message_id": last_mid,
                "last_output_tokens": last_out,
                "last_prompt_count": last_prompt_count
            }),
        );
        for (key, val) in sub_updated {
            cursors.insert(key, val);
        }
        write_cursors(dir, &cursors);
        let _ = fs::remove_file(&lock);
        return 0;
    }

    let sessions = vec![session];
    let payload = build_payload(&sessions);
    dump_sync_payload(dir, "single", &payload);
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();

    let session_ref = &sessions[0];
    let tool_count: u32 = session_ref.tools.values().sum();
    sync_log(dir, &format!(
        "[single] {} prompts={} tools={} tokens_in={} tokens_out={} event={}",
        &session_ref.session_id[..session_ref.session_id.len().min(12)],
        session_ref.prompt_count, tool_count,
        session_ref.total_input_tokens, session_ref.total_output_tokens, event_name
    ));

    let local_sync = crate::config::config_get_bool(dir, "localSync");

    let sync_ok = if local_sync {
        // Write payload to local file instead of POSTing
        let sync_dir = dir.join("local-sync");
        let _ = fs::create_dir_all(&sync_dir);
        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let sid_short = &session_ref.session_id[..session_ref.session_id.len().min(12)];
        let out_path = sync_dir.join(format!("{sid_short}_{ts}.json"));
        match serde_json::to_string_pretty(&payload) {
            Ok(pretty) => {
                if fs::write(&out_path, &pretty).is_ok() {
                    sync_log(dir, &format!("[single] Local sync OK: {}", out_path.display()));
                    true
                } else {
                    sync_log(dir, "[single] Local sync FAILED: write error");
                    false
                }
            }
            Err(e) => {
                sync_log(dir, &format!("[single] Local sync FAILED: {e}"));
                false
            }
        }
    } else {
        let url = format!("{api_base}/sync");
        let result = http_post(&url, &payload_str, Some(&api_key));
        match result {
            Ok((200, resp)) => {
                sync_log(dir, &format!("[single] Sync OK: {resp}"));
                true
            }
            Ok((code, resp)) => {
                sync_log(dir, &format!("[single] Sync FAILED (HTTP {code}): {resp}"));
                false
            }
            Err(e) => {
                sync_log(dir, &format!("[single] Sync FAILED: {e}"));
                false
            }
        }
    };

    let rc = if sync_ok {
        // Use actual max prompt_index from parsed data (includes compaction indices)
        let new_prompt_count = session_ref.prompts.iter()
            .map(|p| p.prompt_index)
            .max()
            .map(|idx| idx + 1)
            .unwrap_or(last_prompt_count);
        cursors.insert(
            transcript_path_str.to_string(),
            json!({
                "byte_offset": new_offset,
                "session_id": cursor_val.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                "project": cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "path_hash": cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or(""),
                "last_request_id": last_rid,
                "last_message_id": last_mid,
                "last_output_tokens": last_out,
                "last_prompt_count": new_prompt_count
            }),
        );
        for (key, val) in sub_updated {
            cursors.insert(key, val);
        }
        write_cursors(dir, &cursors);
        0
    } else {
        1
    };

    let _ = fs::remove_file(&lock);
    rc
}

/// Dry-run transcript aggregation: parses all cursored transcripts and dumps
/// the results to `transcript-debug/` as timestamped JSON files.
/// Does NOT advance cursors or sync to server.
pub fn dump_transcript_debug(dir: &Path) {
    let cursors = read_cursors(dir);
    if cursors.is_empty() {
        return;
    }

    let debug_dir = dir.join("transcript-debug");
    let _ = fs::create_dir_all(&debug_dir);

    let mut sessions: Vec<Session> = Vec::new();
    let mut cursor_snapshots: Vec<serde_json::Value> = Vec::new();

    for (transcript_key, cursor_val) in &cursors {
        let transcript_path = Path::new(transcript_key.as_str());
        if !transcript_path.exists() {
            continue;
        }

        let byte_offset = cursor_val.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let prev_request_id = cursor_val.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_message_id = cursor_val.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_output_tokens = cursor_val.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let fallback_project = cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
        let fallback_path_hash = cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or("");

        let file_size = fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(0);

        cursor_snapshots.push(json!({
            "transcript": transcript_key,
            "file_size": file_size,
            "byte_offset": byte_offset,
            "prev_request_id": prev_request_id,
            "prev_message_id": prev_message_id,
            "prev_output_tokens": prev_output_tokens,
            "new_bytes": if file_size > byte_offset { file_size - byte_offset } else { 0 },
        }));

        if file_size == 0 || file_size == byte_offset {
            continue;
        }

        let parse_offset = if file_size < byte_offset { 0 } else { byte_offset };
        let parse_prev_rid = if file_size < byte_offset { "" } else { prev_request_id };
        let parse_prev_mid = if file_size < byte_offset { "" } else { prev_message_id };
        let parse_prev_out = if file_size < byte_offset { 0 } else { prev_output_tokens };

        if let Some((mut session, new_offset, last_rid, last_mid, last_out)) = parse_transcript_from_offset(
            transcript_path,
            parse_offset,
            parse_prev_rid,
            parse_prev_mid,
            parse_prev_out,
            fallback_project,
            fallback_path_hash,
            0,
        ) {
            cursor_snapshots.last_mut().map(|c| {
                c["parsed_new_offset"] = json!(new_offset);
                c["parsed_last_rid"] = json!(last_rid);
                c["parsed_last_mid"] = json!(last_mid);
                c["parsed_last_out"] = json!(last_out);
            });

            // Discover and parse subagent files
            let subagent_files = find_subagent_files(transcript_path);
            for sub_path in &subagent_files {
                if let Some((sub_session, _, _, _, _)) = parse_transcript_from_offset(
                    sub_path, 0, "", "", 0, fallback_project, fallback_path_hash, 0,
                ) {
                    merge_subagent_sessions(&mut session, sub_session);
                }
            }

            sessions.push(session);
        }
    }

    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let debug_file = debug_dir.join(format!("dump_{ts}.json"));

    let session_data: Vec<serde_json::Value> = sessions.iter().map(|s| {
        let tool_count: u32 = s.tools.values().sum();
        let subagent_count = s.requests.iter().filter(|r| r.is_subagent).count();
        json!({
            "session_id": s.session_id,
            "project": s.project,
            "path_hash": s.path_hash,
            "model": s.model,
            "started_at": s.started_at,
            "ended_at": s.ended_at,
            "message_count": s.message_count,
            "prompt_count": s.prompt_count,
            "tool_count": tool_count,
            "tools": s.tools,
            "request_count": s.requests.len(),
            "subagent_request_count": subagent_count,
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "total_cache_read_tokens": s.total_cache_read_tokens,
            "total_cache_creation_tokens": s.total_cache_creation_tokens,
            "total_turn_duration_ms": s.total_turn_duration_ms,
            "turn_count": s.turn_count,
            "permission_mode": s.permission_mode,
        })
    }).collect();

    let payload = build_payload(&sessions);

    let dump = json!({
        "dumped_at": Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "cursor_count": cursors.len(),
        "sessions_parsed": sessions.len(),
        "cursors": cursor_snapshots,
        "sessions": session_data,
        "sync_payload": payload,
    });

    if let Ok(pretty) = serde_json::to_string_pretty(&dump) {
        if let Ok(mut f) = fs::File::create(&debug_file) {
            let _ = f.write_all(pretty.as_bytes());
        }
    }

    sync_log(dir, &format!(
        "[transcript-debug] Dumped {} sessions to {}",
        sessions.len(),
        debug_file.file_name().unwrap_or_default().to_string_lossy()
    ));
}

/// Dry-run sync: parse all cursored transcripts and show what would be synced.
/// Does NOT advance cursors or POST to backend.
pub fn cmd_sync_dry(dir: &Path, project_filter: Option<&str>) -> i32 {
    let cursors = read_cursors(dir);
    if cursors.is_empty() {
        eprintln!("No transcript cursors found. Run a sync or import first.");
        return 1;
    }

    let mut sessions: Vec<Session> = Vec::new();

    for (transcript_key, cursor_val) in &cursors {
        let transcript_path = Path::new(transcript_key.as_str());
        if !transcript_path.exists() {
            continue;
        }

        let byte_offset = cursor_val.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let prev_request_id = cursor_val.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_message_id = cursor_val.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("");
        let prev_output_tokens = cursor_val.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let fallback_project = cursor_val.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
        let fallback_path_hash = cursor_val.get("path_hash").and_then(|v| v.as_str()).unwrap_or("");

        let file_size = fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(0);
        if file_size == 0 || file_size == byte_offset {
            continue;
        }

        let parse_offset = if file_size < byte_offset { 0 } else { byte_offset };
        let parse_prev_rid = if file_size < byte_offset { "" } else { prev_request_id };
        let parse_prev_mid = if file_size < byte_offset { "" } else { prev_message_id };
        let parse_prev_out = if file_size < byte_offset { 0 } else { prev_output_tokens };

        if let Some((mut session, _, _, _, _)) = parse_transcript_from_offset(
            transcript_path,
            parse_offset,
            parse_prev_rid,
            parse_prev_mid,
            parse_prev_out,
            fallback_project,
            fallback_path_hash,
            0,
        ) {
            // Discover and parse subagent files
            let subagent_files = find_subagent_files(transcript_path);
            for sub_path in &subagent_files {
                let sub_key = sub_path.to_string_lossy().to_string();
                let (sub_offset, sub_prev_rid, sub_prev_mid, sub_prev_out) =
                    if let Some(sub_cursor) = cursors.get(&sub_key) {
                        (
                            sub_cursor.get("byte_offset").and_then(|v| v.as_u64()).unwrap_or(0),
                            sub_cursor.get("last_request_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            sub_cursor.get("last_message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            sub_cursor.get("last_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        )
                    } else {
                        (0u64, String::new(), String::new(), 0u64)
                    };

                let sub_file_size = fs::metadata(sub_path).map(|m| m.len()).unwrap_or(0);
                if sub_file_size == 0 || sub_file_size == sub_offset {
                    continue;
                }

                if let Some((sub_session, _, _, _, _)) = parse_transcript_from_offset(
                    sub_path, sub_offset, &sub_prev_rid, &sub_prev_mid, sub_prev_out,
                    fallback_project, fallback_path_hash, 0,
                ) {
                    merge_subagent_sessions(&mut session, sub_session);
                }
            }

            sessions.push(session);
        }
    }

    // Optionally filter by project name
    if let Some(filter) = project_filter {
        let filter_lower = filter.to_lowercase();
        sessions.retain(|s| s.project.to_lowercase().contains(&filter_lower));
    }

    if sessions.is_empty() {
        eprintln!("No new data to sync.");
        return 0;
    }

    let total_requests: usize = sessions.iter().map(|s| s.requests.len()).sum();
    let subagent_requests: usize = sessions.iter()
        .flat_map(|s| s.requests.iter())
        .filter(|r| r.is_subagent)
        .count();
    let total_input: u64 = sessions.iter().map(|s| s.total_input_tokens).sum();
    let total_output: u64 = sessions.iter().map(|s| s.total_output_tokens).sum();
    let projects: std::collections::HashSet<&str> = sessions.iter().map(|s| s.project.as_str()).collect();

    let payload = build_payload(&sessions);
    let payload_path = dir.join("dry-run-sync.json");
    if let Ok(pretty) = serde_json::to_string_pretty(&payload) {
        let _ = fs::write(&payload_path, &pretty);
    }

    eprintln!("Dry-run sync preview:");
    eprintln!("  Sessions:    {}", sessions.len());
    if subagent_requests > 0 {
        eprintln!("  Requests:    {} ({} subagent)", total_requests, subagent_requests);
    } else {
        eprintln!("  Requests:    {}", total_requests);
    }
    eprintln!("  Input tokens:  {}", total_input);
    eprintln!("  Output tokens: {}", total_output);
    let project_list: Vec<&str> = projects.into_iter().collect();
    eprintln!("  Projects:    {}", project_list.join(", "));
    eprintln!("  Payload:     {}", payload_path.display());

    0
}
