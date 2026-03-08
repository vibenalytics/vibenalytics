#!/bin/bash
set -e

REPO="vibenalytics/vibenalytics"
INSTALL_DIR="${VIBENALYTICS_INSTALL_DIR:-$HOME/.local/bin}"

# Check for required dependencies
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then
  DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER="wget"
else
  echo "Error: curl or wget is required" >&2
  exit 1
fi

download() {
  local url="$1" output="$2"
  if [ "$DOWNLOADER" = "curl" ]; then
    if [ -n "$output" ]; then
      curl -fsSL -o "$output" "$url"
    else
      curl -fsSL "$url"
    fi
  else
    if [ -n "$output" ]; then
      wget -q -O "$output" "$url"
    else
      wget -q -O - "$url"
    fi
  fi
}

# Detect platform
case "$(uname -s)" in
  Darwin) os="darwin" ;;
  Linux)  os="linux" ;;
  *)      echo "Error: unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64)   arch="x64" ;;
  arm64|aarch64)   arch="arm64" ;;
  *)               echo "Error: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

# Detect Rosetta 2 on macOS
if [ "$os" = "darwin" ] && [ "$arch" = "x64" ]; then
  if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]; then
    arch="arm64"
  fi
fi

PLATFORM="vibenalytics-${os}-${arch}"

# Determine version
if [ -n "$1" ]; then
  VERSION="$1"
else
  echo "Fetching latest version..."
  VERSION=$(download "https://api.github.com/repos/${REPO}/releases/latest" "" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
  if [ -z "$VERSION" ]; then
    echo "Error: could not determine latest version" >&2
    exit 1
  fi
fi

echo "Installing vibenalytics ${VERSION} (${os}-${arch})..."

# Download binary
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${PLATFORM}.tar.gz"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/${VERSION}/checksums.json"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

download "$DOWNLOAD_URL" "$TMPDIR/${PLATFORM}.tar.gz" || {
  echo "Error: download failed. Check that ${VERSION} exists for ${PLATFORM}" >&2
  exit 1
}

# Verify checksum
echo "Verifying checksum..."
download "$CHECKSUM_URL" "$TMPDIR/checksums.json" || {
  echo "Warning: could not download checksums, skipping verification" >&2
}

if [ -f "$TMPDIR/checksums.json" ]; then
  expected=$(grep "\"${PLATFORM}\"" "$TMPDIR/checksums.json" | sed -E 's/.*"([a-f0-9]{64})".*/\1/')
  if [ -n "$expected" ]; then
    if [ "$os" = "darwin" ]; then
      actual=$(shasum -a 256 "$TMPDIR/${PLATFORM}.tar.gz" | awk '{print $1}')
    else
      actual=$(sha256sum "$TMPDIR/${PLATFORM}.tar.gz" | awk '{print $1}')
    fi
    if [ "$actual" != "$expected" ]; then
      echo "Error: checksum verification failed" >&2
      echo "  Expected: $expected" >&2
      echo "  Got:      $actual" >&2
      exit 1
    fi
    echo "Checksum OK"
  fi
fi

# Extract and install
tar xzf "$TMPDIR/${PLATFORM}.tar.gz" -C "$TMPDIR"
mkdir -p "$INSTALL_DIR"
mv "$TMPDIR/${PLATFORM}" "$INSTALL_DIR/vibenalytics"
chmod +x "$INSTALL_DIR/vibenalytics"

# Verify installation
if "$INSTALL_DIR/vibenalytics" status >/dev/null 2>&1 || true; then
  echo ""
  echo "Installed vibenalytics ${VERSION} to ${INSTALL_DIR}/vibenalytics"
fi

# Check PATH
if ! echo "$PATH" | tr ':' '\n' | grep -q "^${INSTALL_DIR}$"; then
  echo ""
  echo "Warning: ${INSTALL_DIR} is not in your PATH."
  echo "Add it with:"
  echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc"
  echo ""
fi

# Auto-setup Claude Code plugin (skip if running inside a Claude Code session)
if command -v claude >/dev/null 2>&1 && [ -z "$CLAUDECODE" ]; then
  echo "Setting up Claude Code integration..."

  # Add vibenalytics marketplace if not already registered
  if ! claude plugin marketplace list 2>/dev/null | grep -q "vibenalytics"; then
    claude plugin marketplace add vibenalytics/vibenalytics-claude-plugin 2>/dev/null && \
      echo "  Marketplace added" || \
      echo "  Warning: could not add marketplace"
  fi

  # Install the plugin
  if claude plugin install vibenalytics@vibenalytics 2>/dev/null; then
    echo "  Plugin installed - hooks are active"
  else
    echo "  Warning: plugin install failed"
    echo "  Run manually: claude plugin install vibenalytics@vibenalytics"
  fi
else
  echo ""
  echo "To connect to Claude Code, run:"
  echo "  claude plugin marketplace add vibenalytics/vibenalytics-claude-plugin"
  echo "  claude plugin install vibenalytics@vibenalytics"
fi

echo ""
echo "Get started:"
echo "  vibenalytics              # open dashboard and complete setup"
echo ""
