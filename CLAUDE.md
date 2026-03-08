# Vibenalytics CLI

Native Rust CLI for Claude Code usage analytics. Single binary, zero runtime dependencies.

## Project Structure

```
src/
  main.rs         — CLI entrypoint, arg parsing, command dispatch
  log_cmd.rs      — Hook event handler (stdin → strip → metrics.jsonl)
  sync.rs         — Sync engine (aggregate → POST → archive)
  aggregation.rs  — Session aggregation from JSONL events
  config.rs       — Compile-time constants (API_BASE, APP_NAME, FRONTEND_BASE)
  paths.rs        — Data directory resolution, file path helpers
  auth.rs         — Login/logout (browser-based + credential-based)
  http.rs         — HTTP client (ureq wrapper)
  hash.rs         — FNV-1a path hashing
  projects.rs     — Project registry (enable/disable/filter)
  transcripts.rs  — Claude transcript parser
  import.rs       — History import from ~/.claude/
  update.rs       — Self-update command
  tui/            — Interactive TUI dashboard (ratatui + crossterm)
Cargo.toml        — Package config, dependencies, release profile
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
vibenalytics                                      Launch TUI dashboard (or status if piped)
vibenalytics login                                Browser-based login (opens browser, listens on localhost)
vibenalytics logout                               Clear stored credentials
vibenalytics status                               Show configuration (API base, key, display name)
vibenalytics sync [--force] [--dry] [project]     Aggregate transcripts → POST to backend
vibenalytics import [project] [--dry]             Parse ~/.claude/ transcripts → sync (--dry = skip backend)
vibenalytics project list|add|remove|enable|disable  Manage project tracking
vibenalytics settings [list|get|set]              View or change settings (autoSync, localSync, debugMode)
vibenalytics update                               Self-update to latest release
vibenalytics log                                  (internal) Hook handler — reads event JSON from stdin
vibenalytics parse-transcript <file>              (debug, hidden) Parse transcript and print payload JSON
```

## Build Configuration

Three compile-time constants set via environment variables:

```rust
const APP_NAME: &str          // "vibenalytics" | "vibenalytics-dev"  → data dir name
const DEFAULT_API_BASE: &str  // API endpoint URL
const FRONTEND_BASE: &str     // Frontend URL (for login redirect)
```

### Local dev build
```bash
APP_NAME=vibenalytics-dev cargo build --release
# Defaults: API → localhost:3001, data dir → ~/.config/vibenalytics-dev/
```

### Production build (CI)
```bash
APP_NAME=vibenalytics API_BASE=https://api.vibenalytics.dev/api FRONTEND_BASE=https://app.vibenalytics.dev cargo build --release
# Data dir → ~/.config/vibenalytics/
```

### Dev binary setup
```bash
ln -sf target/release/vibenalytics ~/.local/bin/vibenalytics-dev
```

The dev binary (`vibenalytics-dev`) and production binary (`vibenalytics`) use **separate data directories** determined by `APP_NAME` at compile time. They can run side-by-side via the Claude Code plugin system (production plugin from marketplace, dev plugin from local marketplace).

**Always build the dev binary with `APP_NAME=vibenalytics-dev`** so it uses `~/.config/vibenalytics-dev/` for all its data — metrics, settings, sync logs, transcript cursors, and debug dumps. Without this, it falls back to `~/.config/vibenalytics/` and collides with production.

- **Runtime override:** `.sync-config.json` `apiBase` field overrides the compiled `DEFAULT_API_BASE`

## Data Flow

### Hook-based (real-time)
1. Claude Code hooks pipe event JSON to `vibenalytics log` via stdin (async, ~1ms)
2. Content fields are stripped (only metadata/byte counts kept), appended to `metrics.jsonl`
3. Auto-sync triggers on boundary events (`SessionStart`, `SessionEnd`, `Stop`, `UserPromptSubmit`) or buffer >= 10 events
4. `vibenalytics sync` atomically stages `metrics.jsonl` → aggregates → POSTs to `/api/sync`
5. On success, staged file is archived; on failure, data is prepended back for retry

### Transcript-based (alternative)
1. `--use-transcripts` flag reads Claude Code session transcripts directly from `~/.claude/projects/`
2. Cursor state tracked in `transcript-cursors.json` (byte offset per transcript file)
3. Extracts token usage (input/output/cache), turn duration, model info
4. Deduplicates streaming chunks via requestId tracking

## Key Patterns

- **Path hashing:** Workdir paths are FNV-1a hashed (16 hex chars) — raw paths never leave the machine
- **Content stripping:** `strip_field_bytes()` replaces content with `{_bytes, _type}` stubs
- **Command preview:** Bash commands keep first token (binary name) + 80-char preview
- **Atomic staging:** Sync renames `metrics.jsonl` → `metrics.staging.jsonl` before reading to prevent race conditions between concurrent hook processes
- **Lock files:** Both sync paths use lock files with 60s staleness check
- **Data directory:** `data_dir()` uses compile-time `APP_NAME` → `~/.config/{APP_NAME}/`

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
- Sets `APP_NAME`, `API_BASE`, `FRONTEND_BASE` env vars for production build
- Cross-compiles for 4 targets: darwin-arm64, darwin-x64, linux-x64, linux-arm64
- Generates checksums.json
- Publishes release assets to this repo (`vibenalytics/vibenalytics`)
- Uses default `GITHUB_TOKEN` (requires `contents: write` permission)

## Key Conventions

- Binary name: `vibenalytics` (prod) / `vibenalytics-dev` (dev)
- Data directory: `~/.config/{APP_NAME}/` (compile-time)
- Config file: `.sync-config.json` (in data dir)
- Metrics file: `metrics.jsonl` (in data dir)
- Sync log: `sync.log` (in data dir)
- All timestamps: UTC, ISO 8601 format (`%Y-%m-%dT%H:%M:%SZ`)
- API auth: `X-API-Key` header with `clk_`-prefixed key
