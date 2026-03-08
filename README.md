<p align="center">
  <a href="https://vibenalytics.dev">
    <img src="https://vibenalytics.dev/logo-light.png" alt="Vibenalytics" width="200">
  </a>
</p>

# Vibenalytics

Usage analytics for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Track token consumption, session activity, tool usage, and lines of code changed across all your projects.

The source code is public so you can see exactly how the CLI works and what data is collected.

> **Note:** The CLI syncs data to the [Vibenalytics](https://vibenalytics.dev) hosted backend. It is not designed for self-hosting. If you want to use the CLI without syncing to the cloud, enable **Settings > Sync Locally** in the TUI - all data stays on your machine as local JSON files.

## Prerequisites

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and working
- GitHub account connected to Claude Code

## Install

```bash
curl -fsSL https://vibenalytics.dev/install.sh | bash
```

Supports **macOS** and **Linux** (arm64 and x64). Windows users can run it via WSL.

The install script downloads a prebuilt binary to `~/.local/bin/vibenalytics` and sets up the [Claude Code plugin](https://github.com/vibenalytics/vibenalytics-claude-plugin) automatically.

## Getting started

```bash
vibenalytics
```

The interactive TUI walks you through the full setup:

1. **Log in or create an account** - opens your browser for authentication (free)
2. **Choose sync mode** - auto-discover all projects or manually select which ones to sync
3. **Select projects** - pick which Claude Code projects to track
4. **Import history** (optional) - pull in existing sessions from `~/.claude/` so nothing is lost

Once complete, metrics are captured and synced in the background automatically.

View your data at [app.vibenalytics.dev](https://app.vibenalytics.dev) or run `vibenalytics` to open the built-in TUI dashboard.

## Commands

| Command | Description |
|---|---|
| `vibenalytics` | Launch TUI dashboard (or show status if piped) |
| `vibenalytics login` | Authenticate via browser |
| `vibenalytics logout` | Clear stored credentials |
| `vibenalytics status` | Show connection status and sync health |
| `vibenalytics sync` | Manually trigger a sync |
| `vibenalytics sync --dry` | Preview what would be synced |
| `vibenalytics sync --force` | Force sync even if recently synced |
| `vibenalytics sync [project]` | Sync a specific project (substring match) |
| `vibenalytics import [project]` | Import session history from `~/.claude/` |
| `vibenalytics import --dry` | Preview import without syncing |
| `vibenalytics project list` | List tracked projects |
| `vibenalytics project add [path]` | Add a project (defaults to cwd) |
| `vibenalytics project remove [name]` | Remove a project from tracking |
| `vibenalytics project enable [name]` | Resume syncing for a paused project |
| `vibenalytics project disable [name]` | Pause syncing without removing |
| `vibenalytics settings` | View all settings |
| `vibenalytics settings get <key>` | Get a setting value |
| `vibenalytics settings set <key> <value>` | Change a setting |
| `vibenalytics update` | Self-update to latest release |

## How it works

1. The [Claude Code plugin](https://github.com/vibenalytics/vibenalytics-claude-plugin) hooks into session lifecycle events (start, stop, tool use, prompts, etc.)
2. On sync boundary events, the CLI reads Claude Code's transcript files from `~/.claude/projects/`
3. Only **metadata is extracted** - no prompts, file contents, or tool outputs are read
4. Aggregated session data is posted to the Vibenalytics backend

Data flows through two paths:
- **Real-time sync** - the plugin triggers `vibenalytics log` on hook events, which auto-syncs on session boundaries
- **History import** - `vibenalytics import` parses existing transcripts for backfill

## Privacy and data transparency

No content from your codebase or conversations ever leaves your machine.

### What IS sent

- Session timing (start, end, duration)
- Token counts (input, output, cache read, cache creation)
- Tool names and usage counts (e.g. "Read: 5, Edit: 3")
- Lines of code changed, grouped by file extension (e.g. 80 lines added and 30 removed in `.rs` files, 40 added and 15 removed in `.md` files). Only the extension and line counts are sent - no file names or paths
- Prompt count and character length (not content)
- Model name, Claude Code version, permission mode
- Project directory name (e.g. `my-project`, not the full path)
- Working directory hash (FNV-1a, 16 hex chars - raw path never sent)

### What is NOT sent

| Data | What happens |
|---|---|
| Prompt text / user messages | Only `prompt_length` (character count) is sent |
| File contents | Never read - only line counts from structured diffs |
| Tool outputs | Never read |
| File paths | Only the extension is extracted (e.g. `rs`, `md`) for language stats |
| Working directory | Hashed locally with FNV-1a - only the hash is sent |
| Bash commands | Never sent |

### Full sync payload structure

<details>
<summary>Click to expand the exact JSON structure sent to the backend</summary>

```jsonc
{
  "sessions": [{
    // Session identification
    "session_id": "abc123-...",
    "project_hash": "a1b2c3d4e5f6g7h8",   // FNV-1a hash of workdir path (raw path never sent)
    "project_name": "my-project",            // directory name only (not full path)

    // Timing
    "started_at": "2026-01-15T10:00:00Z",
    "ended_at": "2026-01-15T11:30:00Z",
    "duration_seconds": 5400,

    // Session metadata
    "model": "claude-sonnet-4-5-20250514",
    "claude_version": "2.1.70",
    "cli_version": "0.7.9",
    "permission_mode": "default",
    "prompt_count": 12,
    "message_count": 45,

    // Token usage (aggregated across all requests)
    "total_input_tokens": 150000,
    "total_output_tokens": 35000,
    "total_cache_read_tokens": 500000,
    "total_cache_creation_tokens": 80000,

    // Lines of code changed
    "total_lines_added": 120,
    "total_lines_removed": 45,
    "lines_by_extension": {
      "rs": { "added": 80, "removed": 30 },
      "md": { "added": 40, "removed": 15 }
    },

    // Per-prompt breakdown
    "prompts": [{
      "prompt_index": 0,
      "timestamp": "2026-01-15T10:00:05Z",
      "type": "prompt",
      "model": "claude-sonnet-4-5-20250514",
      "prompt_length": 255,
      "request_count": 5,
      "subagent_count": 1,
      "skills": ["/frontend-design"],
      "command": "/compact",

      "input_tokens": 12000,
      "output_tokens": 3500,
      "cache_read_tokens": 40000,
      "cache_creation_tokens": 8000,
      "context_tokens": 52000,

      "requests": [{
        "request_id": "req_...",
        "message_id": "msg_...",
        "timestamp": "2026-01-15T10:00:05Z",
        "model": "claude-sonnet-4-5-20250514",
        "is_subagent": false,

        "input_tokens": 2400,
        "output_tokens": 700,
        "cache_read_tokens": 8000,
        "cache_creation_tokens": 1600,

        "tools": { "Read": 2, "Edit": 1, "Bash": 1 },

        "lines_added": 15,
        "lines_removed": 3,
        "lines_by_extension": {
          "rs": { "added": 15, "removed": 3 }
        }
      }]
    }]
  }]
}
```

</details>

## Building from source

> **Note:** The CLI requires the Vibenalytics backend to function. Building from source is useful for inspecting the code, contributing, or running with **Sync Locally** enabled (no backend needed).

### Prerequisites

- Rust 2021 edition ([rustup](https://rustup.rs/) recommended)

### Build

```bash
cargo build --release
# Binary: target/release/vibenalytics
```

The default API base is `http://localhost:3001/api`. For production builds the CI sets environment variables at compile time:

```bash
API_BASE=https://api.vibenalytics.dev/api FRONTEND_BASE=https://app.vibenalytics.dev cargo build --release
```

### Local testing

```bash
ln -sf "$(pwd)/target/release/vibenalytics" ~/.local/bin/vibenalytics
```

### Architecture

Single binary, modular source:

```
src/
  main.rs          - CLI entrypoint (clap derive)
  config.rs        - Config read/write, compile-time API_BASE
  auth.rs          - Browser-based OAuth login
  sync.rs          - Aggregation + POST to backend
  import.rs        - Parse ~/.claude/ transcripts, batch sync
  transcripts.rs   - Session/project discovery, JSONL parsing
  aggregation.rs   - Session struct, metrics aggregation
  log_cmd.rs       - Hook event logging (stdin - metrics.jsonl)
  update.rs        - Self-update from GitHub releases
  hash.rs          - FNV-1a path hashing
  paths.rs         - Data directory + file path resolution
  http.rs          - HTTP helpers (ureq)
  tui/             - Interactive TUI dashboard (ratatui + crossterm)
```

### Releasing

1. Tag a version:
   ```bash
   git tag v2.0.0
   git push origin main --tags
   ```
2. GitHub Actions cross-compiles for 4 platforms and creates a release

| Artifact | Target |
|---|---|
| `vibenalytics-darwin-arm64` | macOS Apple Silicon |
| `vibenalytics-darwin-x64` | macOS Intel |
| `vibenalytics-linux-x64` | Linux x64 |
| `vibenalytics-linux-arm64` | Linux ARM64 |

## Links

- [Dashboard](https://app.vibenalytics.dev)
- [Claude Code Plugin](https://github.com/vibenalytics/vibenalytics-claude-plugin)
- [Documentation](https://docs.vibenalytics.dev)
