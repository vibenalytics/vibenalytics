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
API_BASE=https://api.vibenalytics.dev/api FRONTEND_BASE=https://vibenalytics.dev cargo build --release
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

Single binary, two source files:

- `src/main.rs` — CLI entrypoint, hook logging, aggregation, sync, auth, history import
- `src/tui.rs` — Interactive terminal dashboard (ratatui)

### Key data flow

```
Claude Code hooks → vibenalytics log → metrics.jsonl → vibenalytics sync → backend API
```

### Configuration

Config lives in `.sync-config.json` next to the binary (resolved via symlink):
- `apiBase` — backend URL (overrides compiled default)
- `apiKey` — API key for sync authentication
- `displayName` — user display name
