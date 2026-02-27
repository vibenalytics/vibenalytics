use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use serde_json::{json, Value};
use crate::config::{config_get, DEFAULT_API_BASE};
use crate::paths::claude_dir;
use crate::http::http_post;
use crate::aggregation::{build_payload, Session};
use crate::transcripts::{discover_sessions, parse_transcript_from_offset, find_subagent_files, merge_subagent_sessions, read_cursors, write_cursors};

pub enum ImportProgress {
    Parsing { total_files: usize },
    Syncing { batch: usize, total_batches: usize },
    Done(String),
}

/// Start import in a background thread with progress reporting and rate limiting.
pub fn start_import(
    dir: PathBuf,
    selected_dirs: HashSet<String>,
) -> mpsc::Receiver<ImportProgress> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let msg = match do_import_bg(&dir, &selected_dirs, &tx) {
            Ok(m) => m,
            Err(e) => format!("Error: {e}"),
        };
        let _ = tx.send(ImportProgress::Done(msg));
    });
    rx
}

fn do_import_bg(
    dir: &Path,
    selected_dirs: &HashSet<String>,
    tx: &mpsc::Sender<ImportProgress>,
) -> Result<String, String> {
    let claude = claude_dir();
    if !claude.exists() {
        return Err("~/.claude not found".to_string());
    }

    let discovered = discover_sessions(&claude, Some(selected_dirs));
    // Filter to parent sessions only (subagents are merged into parents)
    let parent_sessions: Vec<_> = discovered.iter()
        .filter(|(_, _, _, is_sub, _)| !*is_sub)
        .collect();
    if parent_sessions.is_empty() {
        return Err("No session files found in ~/.claude/projects/".to_string());
    }

    let _ = tx.send(ImportProgress::Parsing { total_files: parent_sessions.len() });

    let output_path = dir.join("history-import.jsonl");
    let mut out_file = fs::File::create(&output_path)
        .map_err(|e| format!("Cannot create output: {e}"))?;

    let mut total_sessions = 0u32;
    let mut total_prompts = 0u32;
    let mut all_sessions: Vec<Session> = Vec::new();
    let mut cursors = read_cursors(dir);

    for (project_name, ph, path, _, _) in &parent_sessions {
        let (mut session, new_offset, last_rid, last_mid, last_out) =
            match parse_transcript_from_offset(path, 0, "", "", 0, project_name, ph, 0) {
                Some(r) => r,
                None => continue,
            };

        // Discover and merge subagent files
        let subagent_files = find_subagent_files(path);
        for sub_path in &subagent_files {
            if let Some((sub_session, sub_offset, sub_rid, sub_mid, sub_out)) =
                parse_transcript_from_offset(sub_path, 0, "", "", 0, project_name, ph, 0)
            {
                merge_subagent_sessions(&mut session, sub_session);
                let sub_key = sub_path.to_string_lossy().to_string();
                if !cursors.contains_key(&sub_key) {
                    cursors.insert(sub_key, json!({
                        "byte_offset": sub_offset,
                        "session_id": session.session_id,
                        "project": session.project,
                        "path_hash": session.path_hash,
                        "last_request_id": sub_rid,
                        "last_message_id": sub_mid,
                        "last_output_tokens": sub_out
                    }));
                }
            }
        }

        total_sessions += 1;
        total_prompts += session.prompt_count;

        // Register cursor with accurate dedup state from the parse
        let path_str = path.to_string_lossy().to_string();
        if !cursors.contains_key(&path_str) {
            cursors.insert(path_str, json!({
                "byte_offset": new_offset,
                "session_id": session.session_id,
                "project": session.project,
                "path_hash": session.path_hash,
                "last_request_id": last_rid,
                "last_message_id": last_mid,
                "last_output_tokens": last_out
            }));
        }

        // Write to history-import.jsonl for debug/audit
        let obj = json!({
            "session_id": session.session_id,
            "project_name": session.project,
            "model": session.model,
            "prompt_count": session.prompt_count,
            "total_input_tokens": session.total_input_tokens,
            "total_output_tokens": session.total_output_tokens,
            "total_cache_read_tokens": session.total_cache_read_tokens,
            "total_cache_creation_tokens": session.total_cache_creation_tokens,
        });
        let _ = writeln!(out_file, "{}", serde_json::to_string(&obj).unwrap_or_default());

        all_sessions.push(session);
    }

    write_cursors(dir, &cursors);
    let parsed_msg = format!("Parsed {total_sessions} sessions, {total_prompts} prompts");

    let api_key = match config_get(dir, "apiKey") {
        Some(k) => k,
        None => return Ok(format!("{parsed_msg} (saved locally, not synced — no API key)")),
    };
    let api_base = DEFAULT_API_BASE;

    let chunks: Vec<&[Session]> = all_sessions.chunks(50).collect();
    let num_batches = chunks.len();
    let mut total_added = 0u32;
    let mut total_skipped = 0u32;

    for (i, chunk) in chunks.iter().enumerate() {
        let _ = tx.send(ImportProgress::Syncing { batch: i + 1, total_batches: num_batches });

        match post_sync_batch(api_base, &api_key, chunk) {
            Ok((added, updated)) => {
                total_added += added;
                total_skipped += updated;
            }
            Err(e) => {
                return Ok(format!("{parsed_msg} (sync failed: {e})"));
            }
        }

        if i + 1 < num_batches {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    Ok(format!("{parsed_msg} — synced {total_added} new, {total_skipped} updated"))
}

fn post_sync_batch(api_base: &str, api_key: &str, sessions: &[Session]) -> Result<(u32, u32), String> {
    let payload = build_payload(sessions);
    let payload_str = serde_json::to_string(&payload).map_err(|e| format!("{e}"))?;
    let url = format!("{api_base}/sync");
    let (status, body) = http_post(&url, &payload_str, Some(api_key))?;
    if status != 200 {
        return Err(format!("HTTP {status}: {body}"));
    }
    let resp: Value = serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;
    let added = resp.pointer("/data/added").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let updated = resp.pointer("/data/updated").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    Ok((added, updated))
}

pub fn cmd_import(dir: &Path, project_filter: Option<&str>, do_sync: bool) -> i32 {
    let claude = claude_dir();
    if !claude.exists() {
        eprintln!("Claude data directory not found: {}", claude.display());
        return 1;
    }

    eprintln!("Scanning {}...", claude.join("projects").display());
    let discovered = discover_sessions(&claude, None);
    // Filter to parent sessions only
    let mut parent_sessions: Vec<_> = discovered.into_iter()
        .filter(|(_, _, _, is_sub, _)| !*is_sub)
        .collect();

    if let Some(filter) = project_filter {
        let filter_lower = filter.to_lowercase();
        let before = parent_sessions.len();
        parent_sessions.retain(|(name, _, _, _, _)| name.to_lowercase().contains(&filter_lower));
        eprintln!("Filter '{}': {} of {} session files match", filter, parent_sessions.len(), before);
    }

    if parent_sessions.is_empty() {
        eprintln!("No session files found.");
        return 1;
    }

    eprintln!("Found {} session files. Parsing...", parent_sessions.len());

    let output_path = dir.join("history-import.jsonl");
    let mut out_file = match fs::File::create(&output_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Cannot create output file: {e}");
            return 1;
        }
    };

    let mut total_sessions = 0u32;
    let mut total_prompts = 0u32;
    let mut total_tools = 0u32;
    let mut projects: HashSet<String> = HashSet::new();
    let mut earliest = String::new();
    let mut latest = String::new();
    let mut all_sessions: Vec<Session> = Vec::new();
    let mut cursors = read_cursors(dir);

    for (i, (project_name, ph, path, _, _)) in parent_sessions.iter().enumerate() {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        eprint!("\r  [{}/{}] Parsing {}...", i + 1, parent_sessions.len(), file_name);

        let (mut session, new_offset, last_rid, last_mid, last_out) =
            match parse_transcript_from_offset(path, 0, "", "", 0, project_name, ph, 0) {
                Some(r) => r,
                None => continue,
            };

        // Discover and merge subagent files
        let subagent_files = find_subagent_files(path);
        for sub_path in &subagent_files {
            if let Some((sub_session, sub_offset, sub_rid, sub_mid, sub_out)) =
                parse_transcript_from_offset(sub_path, 0, "", "", 0, project_name, ph, 0)
            {
                merge_subagent_sessions(&mut session, sub_session);
                let sub_key = sub_path.to_string_lossy().to_string();
                if !cursors.contains_key(&sub_key) {
                    cursors.insert(sub_key, json!({
                        "byte_offset": sub_offset,
                        "session_id": session.session_id,
                        "project": session.project,
                        "path_hash": session.path_hash,
                        "last_request_id": sub_rid,
                        "last_message_id": sub_mid,
                        "last_output_tokens": sub_out
                    }));
                }
            }
        }

        total_sessions += 1;
        total_prompts += session.prompt_count;
        total_tools += session.tools.values().sum::<u32>();
        projects.insert(session.project.clone());

        // Register cursor with accurate dedup state from the parse
        let path_str = path.to_string_lossy().to_string();
        if !cursors.contains_key(&path_str) {
            cursors.insert(path_str, json!({
                "byte_offset": new_offset,
                "session_id": session.session_id,
                "project": session.project,
                "path_hash": session.path_hash,
                "last_request_id": last_rid,
                "last_message_id": last_mid,
                "last_output_tokens": last_out
            }));
        }

        if !session.started_at.is_empty() {
            if earliest.is_empty() || session.started_at < earliest {
                earliest = session.started_at.clone();
            }
            if latest.is_empty() || session.ended_at > latest {
                latest = session.ended_at.clone();
            }
        }

        // Write to history-import.jsonl for debug/audit
        let obj = json!({
            "session_id": session.session_id,
            "project_name": session.project,
            "model": session.model,
            "prompt_count": session.prompt_count,
            "total_input_tokens": session.total_input_tokens,
            "total_output_tokens": session.total_output_tokens,
            "total_cache_read_tokens": session.total_cache_read_tokens,
            "total_cache_creation_tokens": session.total_cache_creation_tokens,
        });
        let _ = writeln!(out_file, "{}", serde_json::to_string(&obj).unwrap_or_default());

        all_sessions.push(session);
    }

    write_cursors(dir, &cursors);

    eprintln!("\r                                                              ");

    eprintln!("Done!");
    eprintln!("  Sessions:  {total_sessions}");
    eprintln!("  Projects:  {}", projects.len());
    eprintln!("  Prompts:   {total_prompts}");
    eprintln!("  Tool calls:{total_tools}");
    if !earliest.is_empty() {
        eprintln!("  Range:     {} .. {}", &earliest[..10.min(earliest.len())], &latest[..10.min(latest.len())]);
    }
    eprintln!("  Output:    {}", output_path.display());

    if !do_sync {
        // Dry mode: write the exact sync payload that would be POSTed
        let payload = build_payload(&all_sessions);
        let payload_path = dir.join("dry-run-payload.json");
        if let Ok(pretty) = serde_json::to_string_pretty(&payload) {
            if fs::write(&payload_path, &pretty).is_ok() {
                eprintln!("  Payload:   {}", payload_path.display());
            }
        }
    }

    if do_sync {
        let api_key = match config_get(dir, "apiKey") {
            Some(key) => key,
            None => {
                eprintln!("\nNo API key configured. Run: vibenalytics login");
                return 1;
            }
        };
        let api_base = DEFAULT_API_BASE;

        eprintln!("\nSyncing history...");

        let chunks: Vec<&[Session]> = all_sessions.chunks(50).collect();
        let num_batches = chunks.len();
        let mut total_added = 0u32;
        let mut total_updated = 0u32;

        for (batch_idx, chunk) in chunks.iter().enumerate() {
            eprint!("\r  Batch {}/{} ({} sessions)...", batch_idx + 1, num_batches, chunk.len());
            match post_sync_batch(api_base, &api_key, chunk) {
                Ok((added, updated)) => {
                    total_added += added;
                    total_updated += updated;
                }
                Err(e) => {
                    eprintln!("\n  Batch {} failed: {}", batch_idx + 1, e);
                    eprintln!("  {} sessions synced before failure.", total_added);
                    return 1;
                }
            }
            if batch_idx + 1 < num_batches {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }

        eprintln!("\r                                                  ");
        eprintln!("Sync complete!");
        eprintln!("  Added:   {total_added}");
        eprintln!("  Updated: {total_updated}");
    }

    0
}
