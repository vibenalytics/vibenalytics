# TODO - Sync Pipeline

Known issues and improvements for the sync pipeline.

## Bugs

### prompt_index vs prompt_count mismatch
- **Severity:** High
- **Files:** `transcripts.rs`, `sync.rs`
- The cursor stores `last_prompt_count` (real user prompts only) but uses it as `prompt_index_offset` (which should include compaction indices). After a compaction, prompt indices can collide because compaction increments `current_prompt_index` but not `session.prompt_count`.
- **Fix:** Track the actual last `prompt_index` in the cursor instead of just the count of real prompts.

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
