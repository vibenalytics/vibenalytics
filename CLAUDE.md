# Vibenalytics CLI

Native Rust CLI for Claude Code usage analytics. Single binary, zero runtime dependencies.

## Project Structure

```
src/
  main.rs    — CLI entrypoint, all commands, hook logging, aggregation, sync, auth
  tui.rs     — Interactive TUI dashboard (ratatui + crossterm)
Cargo.toml   — Package config, dependencies, release profile
```

## Tech Stack

- **Language:** Rust 2021 edition
- **HTTP:** ureq 2 (sync, minimal)
- **JSON:** serde + serde_json
- **Time:** chrono (UTC timestamps, ISO 8601 parsing)
- **TUI:** ratatui 0.29 + crossterm 0.28
- **Browser:** open 5 (for `login` command)
- **Release profile:** opt-level=s, LTO, strip (small binary)

## Commands

```
vibenalytics log [--use-transcripts]              Read hook JSON from stdin, strip content, append to metrics.jsonl
vibenalytics sync [--use-transcripts]             Aggregate metrics.jsonl → POST to backend → flush
vibenalytics login                                Browser-based login (opens browser, listens on localhost)
vibenalytics login <email> <password>             Login with credentials → get JWT → generate API key
vibenalytics login --api-key <key>                Set API key directly
vibenalytics status                               Show configuration (API base, key, display name)
vibenalytics aggregate <file>                     Dump aggregated JSON to stdout (debug)
vibenalytics tui                                  Launch interactive TUI dashboard
vibenalytics import-from-history [project] [--dry]  Parse ~/.claude/ transcripts → sync (--dry = JSONL only)
```

## Build Configuration

API base URL is set at **compile time** via environment variable:

```rust
const DEFAULT_API_BASE: &str = match option_env!("API_BASE") {
    Some(url) => url,
    None => "http://localhost:3001/api",
};
```

- **Local dev:** `cargo build --release` (defaults to localhost:3001)
- **Production:** `API_BASE=https://api.vibenalytics.dev/api cargo build --release`
- **Runtime override:** `.sync-config.json` `apiBase` field overrides the compiled default

## Data Flow

### Hook-based (real-time)
1. Claude Code hooks pipe event JSON to `vibenalytics log` via stdin (async, ~1ms)
2. Content fields are stripped (only metadata/byte counts kept), appended to `metrics.jsonl`
3. Auto-sync triggers on boundary events (`SessionStart`, `SessionEnd`, `Stop`, `UserPromptSubmit`) or buffer >= 10 events
4. `vibenalytics sync` aggregates events into per-session summaries, POSTs to `/api/sync`
5. On success, `metrics.jsonl` is archived and flushed

### Transcript-based (alternative)
1. `--use-transcripts` flag reads Claude Code session transcripts directly from `~/.claude/projects/`
2. Cursor state tracked in `transcript-cursors.json` (byte offset per transcript file)
3. Extracts token usage (input/output/cache), turn duration, model info
4. Deduplicates streaming chunks via requestId tracking

## Key Patterns

- **Path hashing:** Workdir paths are FNV-1a hashed (16 hex chars) — raw paths never leave the machine
- **Content stripping:** `strip_field_bytes()` replaces content with `{_bytes, _type}` stubs
- **Command preview:** Bash commands keep first token (binary name) + 80-char preview
- **Lock files:** Both sync paths use lock files with 60s staleness check
- **Config resolution:** `data_dir()` resolves symlinks via `fs::canonicalize()` — config lives next to the real binary

## Auth Flow

1. `login` opens browser to `{frontend}/auth/cli?port={port}`
2. Binary listens on a random localhost TCP port
3. Frontend authenticates user, generates API key, redirects to `localhost:{port}/callback?key=...&name=...`
4. Binary reads the HTTP request, extracts key/name, saves to `.sync-config.json`

## Sync Payload Format

```json
{
  "sessions": [{
    "session_id": "...",
    "project_hash": "abc123...",
    "project_name": "myproject",
    "started_at": "2026-01-01T00:00:00Z",
    "ended_at": "2026-01-01T01:00:00Z",
    "events": {"SessionStart": 1, "PostToolUse": 15},
    "tools": {"Read": 5, "Edit": 3, "Bash": 7},
    "prompt_count": 10,
    "total_input_bytes": 50000,
    "total_response_bytes": 120000,
    "permission_mode": "default",
    "duration_seconds": 3600,
    "tool_latencies": [{"tool": "Bash", "avg_ms": 1500, "count": 7}],
    "total_input_tokens": 100000,
    "total_output_tokens": 25000,
    "model": "claude-opus-4-6"
  }]
}
```

## CI/CD

GitHub Actions (`.github/workflows/release.yml`):
- Triggers on tag push (`v*`)
- Cross-compiles for 4 targets: darwin-arm64, darwin-x64, linux-x64, linux-arm64
- Generates checksums.json
- Publishes release assets to the **public** repo `vibenalytics/vibenalytics-cli`
- Requires `PUBLIC_RELEASE_TOKEN` secret (PAT with `contents: write` on public repo)

## Key Conventions

- Binary name: `vibenalytics` (Cargo.toml `[package] name`)
- Config file: `.sync-config.json` (next to binary, resolved through symlinks)
- Metrics file: `metrics.jsonl` (next to binary)
- Sync log: `sync.log` (next to binary)
- All timestamps: UTC, ISO 8601 format (`%Y-%m-%dT%H:%M:%SZ`)
- API auth: `X-API-Key` header with `clk_`-prefixed key
