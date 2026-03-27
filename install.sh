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

# Extract binary
tar xzf "$TMPDIR/${PLATFORM}.tar.gz" -C "$TMPDIR"

# Versioned install layout:
#   ~/.local/share/vibenalytics/versions/{version}  (binary)
#   ~/.local/bin/vibenalytics -> versions/{version}  (symlink)
VERSIONS_DIR="$HOME/.local/share/vibenalytics/versions"
VERSION_NUM="${VERSION#v}"
VERSION_BIN="$VERSIONS_DIR/$VERSION_NUM"
LINK_PATH="$INSTALL_DIR/vibenalytics"

mkdir -p "$VERSIONS_DIR"
mkdir -p "$INSTALL_DIR"

# Install binary to versions directory
mv "$TMPDIR/${PLATFORM}" "$VERSION_BIN"
chmod +x "$VERSION_BIN"

# Migrate: if existing install is a regular file (not symlink), preserve it
if [ -f "$LINK_PATH" ] && [ ! -L "$LINK_PATH" ]; then
  OLD_VERSION=$("$LINK_PATH" -V 2>/dev/null | awk '{print $NF}' || echo "")
  if [ -n "$OLD_VERSION" ] && [ "$OLD_VERSION" != "$VERSION_NUM" ]; then
    OLD_BIN="$VERSIONS_DIR/$OLD_VERSION"
    if [ ! -f "$OLD_BIN" ]; then
      mv "$LINK_PATH" "$OLD_BIN"
      echo "Migrated existing v${OLD_VERSION} to versions directory"
    else
      rm -f "$LINK_PATH"
    fi
  else
    rm -f "$LINK_PATH"
  fi
fi

# Atomic symlink swap: temp symlink + rename (no gap where binary is missing)
TEMP_LINK="$INSTALL_DIR/.vibenalytics.tmp.$$"
rm -f "$TEMP_LINK"
ln -s "$VERSION_BIN" "$TEMP_LINK"
mv "$TEMP_LINK" "$LINK_PATH"

# Clean up old versions (keep 2 most recent)
cd "$VERSIONS_DIR"
# shellcheck disable=SC2012
ls -t | tail -n +3 | while read -r old; do
  [ "$old" = "$VERSION_NUM" ] && continue
  rm -f "$VERSIONS_DIR/$old"
done
cd - >/dev/null

echo ""
echo "Installed vibenalytics ${VERSION} to ${LINK_PATH}"
echo "  Binary:  ${VERSION_BIN}"
echo "  Symlink: ${LINK_PATH} -> ${VERSION_BIN}"

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
