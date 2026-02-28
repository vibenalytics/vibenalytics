# Sync Pipeline

How Vibenalytics captures, parses, aggregates, and syncs Claude Code usage data.

## Overview

Vibenalytics reads Claude Code's transcript files (append-only JSONL) to extract token usage, tool calls, prompt metadata, and session analytics. Data flows through two paths:

- **Real-time sync** - Claude Code hooks trigger per-prompt syncing via `vibenalytics log`
- **History import** - `vibenalytics import` parses all historical transcripts in batch

Both paths use the same parser (`parse_transcript_from_offset`) and payload builder (`build_payload`), so all enrichment (compaction tracking, context tokens, etc.) applies uniformly.

## When syncing happens

### Hook events

Claude Code fires hook events as JSON to `vibenalytics log` via stdin. The CLI receives these and decides whether to trigger a sync.

**Sync triggers** (boundary events):

| Event | When it fires | Sync action |
|-------|--------------|-------------|
| `Stop` | After assistant finishes responding (including interruptions) | Primary sync trigger - parses new transcript data and syncs |
| `SessionEnd` | When the session is closing | Cleanup sync - captures any remaining data |

**Non-sync events** (cursor registration only):

| Event | When it fires | Action |
|-------|--------------|--------|
| `UserPromptSubmit` | After user sends a prompt | Registers cursor if new session, no sync |
| `SessionStart` | When a session begins | Registers cursor if new session, no sync |
| `PreToolUse` / `PostToolUse` | Before/after tool execution | No action |

**Why only `Stop` and `SessionEnd`?**

`Stop` fires after the assistant's full response is in the transcript. This guarantees each sync contains complete prompt data (user message + assistant response + tool calls). Syncing on `UserPromptSubmit` would split a prompt across two syncs - the user message in one, the response in another - forcing the backend to stitch partial data together.

**Transcript flush delay**

There is a race condition between the `Stop` hook event and transcript file writes. Claude Code fires `Stop` ~500ms before the final assistant message is fully flushed to the JSONL file. Without mitigation, the sync would miss the last response, creating "ghost" prompt entries in the next sync (assistant data with no preceding user message).

To handle this, the CLI sleeps **2 seconds** after receiving a boundary event before parsing the transcript. This ensures all pending writes are complete.

### Manual sync

`vibenalytics sync` iterates ALL cursors in `transcript-cursors.json` and syncs any with new data. This catches orphaned sessions, missed events, or data from killed sessions.

### History import

`vibenalytics import` discovers all transcript files in `~/.claude/projects/`, parses each from offset 0, and syncs in batches of 50 sessions with 500ms delays between batches.

## Transcript parsing

### Source files

Claude Code writes session transcripts to:
```
~/.claude/projects/{project-dir-name}/{session-uuid}.jsonl
```

Each line is a JSON object with a `type` field: `"user"`, `"assistant"`, `"system"`, or `"progress"`.

### Incremental parsing

The parser (`parse_transcript_from_offset`) reads from a byte offset, skipping already-synced data:

```
parse_transcript_from_offset(
    filepath,           // path to .jsonl file
    byte_offset,        // resume position (0 = start)
    prev_request_id,    // last synced request ID (for dedup)
    prev_message_id,    // last synced message ID (for dedup)
    prev_output_tokens, // last output token count (for incremental output)
    fallback_project,   // project name from cursor
    fallback_path_hash, // path hash from cursor
    prompt_index_offset  // starting prompt index (for continuity across syncs)
) -> Option<(Session, new_offset, last_rid, last_mid, last_out)>
```

### Boundary deduplication

When resuming from a cursor, the parser may re-encounter the last request from the previous sync (streaming chunks share the same message ID). To avoid double-counting:

- Constructs a composite key: `{message_id}:{request_id}`
- When the boundary request is found, subtracts `prev_output_tokens` from output (counts only new tokens)
- Zeroes input and cache tokens for that request (already counted in previous sync)

### Prompt detection

User messages are classified as real prompts if they pass `is_real_user_prompt()`:

```rust
fn is_real_user_prompt(content: &str) -> bool {
    !content.is_empty()
        && !content.starts_with("<local-command")
        && !content.starts_with("<bash-")
        && !content.starts_with("/plugin")
}
```

This filters out internal system messages, plugin commands, and empty messages.

### Prompt classification

Each prompt is classified by `classify_prompt()`:

- **`"prompt"`** - Regular user prompt (default)
- **`"command"`** - Slash command (detected by `<command-name>` tag), with the command name extracted
- **`"compaction"`** - Context compaction event (see below)

### Prompt index continuity

Each sync carries a `prompt_index_offset` (stored as `last_prompt_count` in the cursor). This ensures prompt indices are globally unique across incremental syncs:

```
Sync 1: prompt 0, 1, 2     -> cursor stores last_prompt_count = 3
Sync 2: prompt 3, 4         -> cursor stores last_prompt_count = 5
Sync 3: compaction 5, prompt 6  -> cursor stores last_prompt_count = 6 (compaction doesn't count)
```

`prompt_count` only increments for real user prompts, not compaction entries.

## Compaction handling

When Claude Code's context window fills up (~165K tokens), it compacts the conversation. This produces three transcript entries:

1. **System event** - `type: "system"`, `subtype: "compact_boundary"`
   - `compactMetadata.trigger` - `"auto"` or `"manual"`
   - `compactMetadata.preTokens` - context size before compaction (e.g., 168072)

2. **Synthetic user message** - `type: "user"`, `isCompactSummary: true`
   - Contains the compacted conversation summary
   - Not counted as a real prompt

3. **Assistant response** - `type: "assistant"` with token usage
   - Processes the summary, typically ~10 output tokens
   - High `cache_creation_tokens` (re-caching the compacted context)

### How the parser tracks compaction

1. On `compact_boundary` system event: stores `(trigger, pre_tokens)` in `pending_compaction`
2. On `isCompactSummary` user message: flushes the current prompt, creates a new prompt entry with `msg_type = "compaction"`, attaches the pending metadata
3. The assistant response naturally attaches its token usage to the compaction's prompt index

The result: compaction appears as a distinct entry in the `prompts` array with `"type": "compaction"`, separate from real prompt token costs.

## Token aggregation

### Per-request tokens

Each assistant message in the transcript contains an API `usage` object:

| Field | Description |
|-------|-------------|
| `input_tokens` | Non-cached input tokens |
| `cache_read_input_tokens` | Tokens read from prompt cache |
| `cache_creation_input_tokens` | Tokens written to prompt cache |
| `output_tokens` | Generated output tokens |

For streaming responses, the parser tracks the maximum `output_tokens` seen (each chunk reports cumulative output). Input and cache tokens are set from the first chunk only (`is_new` flag).

### Per-prompt aggregation

`build_prompts()` groups requests by `prompt_index` and computes:

- **Token sums** - `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_creation_tokens` summed across all requests in the prompt
- **`context_tokens`** - Total context window size at prompt start. Computed from the **first request** only: `input_tokens + cache_read_tokens + cache_creation_tokens`. This shows how the context grows prompt by prompt
- **Tool counts** - Merged from parser-tracked per-prompt tool usage
- **Request count** - Number of API requests in this prompt

### Per-session totals

Session-level totals are accumulated during parsing:

- `total_input_tokens`, `total_output_tokens`, `total_cache_read_tokens`, `total_cache_creation_tokens`
- `total_turn_duration_ms`, `turn_count` (from `turn_duration` system events)
- `tools` map (tool name -> call count)
- `prompt_count` (real user prompts only, excludes compaction)

### Subagent handling

Claude Code spawns subagent processes (e.g., Task tool agents) that write to separate transcript files:
```
~/.claude/projects/{project}/{session-uuid}/subagents/agent-{n}/{uuid}.jsonl
```

These are discovered via `find_subagent_files()` and merged into the parent session:
- All subagent requests are marked with `is_subagent: true`
- Requests are assigned to parent prompt indices based on timestamp ranges
- `subagent_count` is incremented on affected prompts
- Subagent tokens are added to parent session totals
- Separate `subagent_input_tokens` / `subagent_output_tokens` fields in the payload

## Empty-sync skip

When a sync trigger fires but the parsed data has no meaningful content:

```rust
if session.prompt_count == 0
    && session.total_input_tokens == 0
    && session.total_output_tokens == 0
{
    // Advance cursor without syncing
}
```

The cursor is still advanced (so the same empty region isn't re-parsed), but no POST/local-write occurs.

## Cursor state

Stored in `~/.config/{APP_NAME}/transcript-cursors.json`. One entry per transcript file:

```json
{
    "/path/to/session.jsonl": {
        "byte_offset": 245760,
        "session_id": "9b6fc537-...",
        "project": "my-project",
        "path_hash": "6d5eb8c75aeb3468",
        "last_request_id": "req_011CYXYay...",
        "last_message_id": "msg_01T9L1YUm...",
        "last_output_tokens": 153,
        "last_prompt_count": 12
    }
}
```

Cursor updates are atomic: write to `.json.tmp`, then rename.

## Lock mechanism

A lock file (`transcript-cursors.json.lock`) prevents concurrent sync processes from racing. The lock:
- Contains the process ID
- Has a 60-second staleness check (stale locks are removed)
- If lock is held and not stale, the sync is skipped (data caught on next trigger)

## Local sync (dev mode)

When `localSync` config is `true`, payloads are written to `local-sync/` as JSON files instead of HTTP POST. The cursor is still advanced. Useful for testing without a backend.

```
~/.config/vibenalytics-dev/local-sync/9b6fc537-904_20260226_232641.json
```

## Sync log

All sync activity is logged to `sync.log` in the data directory:

```
[hook] Stop tool=- session=9b6fc537-904 boundary=true
[single] 9b6fc537-904 prompts=1 tools=3 tokens_in=6 tokens_out=1002 event=Stop
[single] Sync OK: {"ok":true,"data":{"added":0,"updated":1}}
```

## Type definitions

TypeScript-equivalent types for the sync payload:

```typescript
interface SyncPayload {
    sessions: SessionPayload[];
}

interface SessionPayload {
    session_id: string;
    project_hash: string;                  // FNV-1a hash of project directory path
    project_name: string;                  // Directory name (e.g., "my-project")
    started_at: string;                    // ISO 8601 UTC
    ended_at: string;                      // ISO 8601 UTC
    duration_seconds?: number;             // Computed from started_at/ended_at
    events: Record<string, number>;        // Hook event counts (empty for transcript sync)
    prompt_count: number;                  // Real user prompts (excludes compaction)
    message_count: number;                 // Total transcript messages parsed
    permission_mode?: string;              // "default" | "acceptEdits" | ...
    model?: string;                        // e.g., "claude-opus-4-6"
    claude_version?: string;               // e.g., "2.1.39"

    // Token totals (across all prompts)
    total_input_tokens?: number;
    total_output_tokens?: number;
    total_cache_read_tokens?: number;
    total_cache_creation_tokens?: number;

    // Turn duration
    total_turn_duration_ms?: number;
    turn_count?: number;

    // Per-prompt breakdown (requests nested inside each prompt)
    prompts?: PromptPayload[];
}

interface PromptPayload {
    prompt_index: number;                  // Globally unique across incremental syncs
    timestamp: string;                     // ISO 8601 UTC - when the prompt started
    type: "prompt" | "command" | "compaction";

    // Token usage (summed across all requests in this prompt)
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cache_creation_tokens: number;

    // Context window size at prompt start (first request's total input)
    context_tokens: number;                // = input + cache_read + cache_creation of first request

    request_count: number;                 // Number of API requests in this prompt

    // Optional fields
    model?: string;                        // Model used
    prompt_text?: string;                  // First 500 chars of user message
    command?: string;                      // Slash command name (when type = "command")
    subagent_count?: number;               // Subagents spawned during this prompt

    // Compaction metadata (when type = "compaction")
    compaction_trigger?: string;           // "auto" | "manual"
    compaction_pre_tokens?: number;        // Context size before compaction

    // Nested per-request breakdown
    requests?: RequestPayload[];           // Individual API requests in this prompt
}

interface RequestPayload {
    request_id: string;                    // Anthropic API request ID
    message_id: string;                    // Anthropic API message ID
    timestamp: string;                     // ISO 8601 UTC
    model: string;                         // e.g., "claude-opus-4-6"
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cache_creation_tokens: number;
    is_subagent: boolean;                  // true if from a subagent process
    tools: Record<string, number>;          // Tool call counts in this request (e.g., {"Read": 2, "Edit": 1})
}

```

## Context growth pattern

The `context_tokens` field reveals how the conversation context grows:

```
Prompt 0:   56,286 tokens   (session start)
Prompt 5:  117,706 tokens   (growing...)
Prompt 10: 158,388 tokens   (approaching limit)
Compact:    46,773 tokens   (reset after compaction)
Prompt 12:  61,894 tokens   (growing again from compressed state)
...
```

This sawtooth pattern repeats every ~10-15 prompts, with compaction firing around 165K tokens and resetting to ~45-57K.
