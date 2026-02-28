# TODO - Sync Pipeline

Known issues and improvements for the sync pipeline.

## Done

### Ghost prompt entries (orphan assistant data)
- **Fixed:** 2s sleep in `log_cmd.rs` + `prompt_started` guard in `transcripts.rs`
- Stop hook fires ~500ms before the final assistant message is flushed to the transcript. This caused "ghost" prompt entries with no `prompt_text` in nearly every sync.
- Two-layer fix: sleep gives the transcript time to flush, and the parser skips PromptUsage entries before the first real user message in a sync window.

### prompt_index vs prompt_count mismatch
- **Fixed:** `sync.rs` cursor now derives `last_prompt_count` from `max(prompt_index) + 1` instead of summing `session.prompt_count`.
- The cursor stored `last_prompt_count` (real user prompts only) but used it as `prompt_index_offset` (which should include compaction indices). After a compaction, prompt indices could collide.

## Improvements

### Sleep only on Stop events
- **Severity:** Medium
- **File:** `log_cmd.rs`
- The 2s transcript flush delay applies to all boundary events including `SessionEnd`. Only `Stop` has the race condition where the final assistant message is written after the hook fires. `SessionEnd` fires during cleanup when all writes are already done.
- **Fix:** Wrap the sleep in `if event_name == "Stop"`.

### Subagent write timing
- **Severity:** Low
- **Files:** `sync.rs`, `transcripts.rs`
- Subagent transcript files might still be written after the 2s sleep window. Currently caught on the next sync trigger, so no data loss - just delayed reporting.
- **Fix (optional):** Add polling/retry for subagent file discovery.

### Missing last_message_id in cursor init
- **Severity:** Low
- **File:** `log_cmd.rs`
- Initial cursor JSON (line ~48) doesn't include `last_message_id`. The sync code handles this gracefully (defaults to `""`), but adding it would be consistent with the cursor schema.
