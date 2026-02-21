# Vibenalytics CLI (Source)

Private source repository for the Vibenalytics CLI — a native Rust binary that logs, aggregates, and syncs Claude Code usage metrics.

Public-facing repo with releases: [vibenalytics/vibenalytics-cli](https://github.com/vibenalytics/vibenalytics-cli)

## Development

### Prerequisites

- Rust 2021 edition (`rustup` recommended)

### Build (local dev)

```bash
cargo build --release
# Binary: target/release/vibenalytics
```

The default API base is `http://localhost:3001/api`. For production builds:

```bash
API_BASE=https://api.vibenalytics.dev/api cargo build --release
```

### Symlink for local testing

```bash
ln -sf "$(pwd)/target/release/vibenalytics" ~/.local/bin/vibenalytics
```

## Releasing

1. Bump version in `Cargo.toml`
2. Commit and tag:
   ```bash
   git tag v2.0.0
   git push origin main --tags
   ```
3. GitHub Actions builds for 4 platforms and publishes to the [public repo](https://github.com/vibenalytics/vibenalytics-cli/releases)

### Required secrets

- `PUBLIC_RELEASE_TOKEN` — GitHub PAT with `contents: write` on `vibenalytics/vibenalytics-cli`

### Targets

| Artifact | Target |
|---|---|
| `vibenalytics-darwin-arm64` | `aarch64-apple-darwin` (macOS Apple Silicon) |
| `vibenalytics-darwin-x64` | `x86_64-apple-darwin` (macOS Intel) |
| `vibenalytics-linux-x64` | `x86_64-unknown-linux-gnu` |
| `vibenalytics-linux-arm64` | `aarch64-unknown-linux-gnu` |

## Architecture

Single binary, modular source:

```
src/
  main.rs          — CLI entrypoint (clap derive)
  config.rs        — Config read/write, compile-time API_BASE
  auth.rs          — Browser-based OAuth login (non-blocking for TUI)
  sync.rs          — Aggregation + POST to backend
  import.rs        — Parse ~/.claude/ transcripts, batch sync
  transcripts.rs   — Session/project discovery, JSONL parsing
  aggregation.rs   — Session struct, metrics aggregation
  log_cmd.rs       — Hook event logging (stdin → metrics.jsonl)
  hash.rs          — FNV-1a path hashing
  paths.rs         — Data directory + file path resolution
  http.rs          — HTTP helpers (ureq)
  tui/
    mod.rs         — TUI app loop, state, event handling
    theme.rs       — Color palette
    header.rs      — Header bar, tab bar, footer
    dashboard.rs   — Overview tab
    sessions.rs    — Sessions tab
    projects.rs    — Projects tab
    settings.rs    — Settings tab (actions)
    import_picker.rs — Project selection for history import
    overlay.rs     — Modal overlays
```

### Key data flow

```
Claude Code hooks → vibenalytics log → metrics.jsonl → vibenalytics sync → backend API
```

### Configuration

Config lives in `.sync-config.json` next to the binary (resolved via symlink):
- `apiBase` — backend URL (overrides compiled default)
- `apiKey` — API key for sync authentication
- `displayName` — user display name
