#!/usr/bin/env bash
# Install the everruns CLI binary.
#
# Modes:
#   1. Pre-built binary from GitHub Release (default, no Rust needed)
#   2. Build from source via `cargo install` (fallback, or --source flag)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/everruns/everruns/main/scripts/install-cli.sh | bash
#   ./scripts/install-cli.sh                  # latest release binary
#   ./scripts/install-cli.sh --version v0.9.0 # specific version
#   ./scripts/install-cli.sh --source         # build from source
#   INSTALL_DIR=~/.local/bin ./scripts/install-cli.sh

set -euo pipefail

REPO="everruns/everruns"
BINARY_NAME="everruns"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
VERSION=""
FROM_SOURCE=false
CURL_OPTS=(--connect-timeout 10 --max-time 120 --retry 3 --retry-delay 2)

# --- Parse args ---
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      if [[ $# -lt 2 ]]; then
        echo "ERROR: --version requires a value (e.g. v0.9.0)" >&2; exit 1
      fi
      VERSION="$2"; shift 2
      ;;
    --source)  FROM_SOURCE=true; shift ;;
    --install-dir)
      if [[ $# -lt 2 ]]; then
        echo "ERROR: --install-dir requires a directory path" >&2; exit 1
      fi
      INSTALL_DIR="$2"; shift 2
      ;;
    -h|--help)
      echo "Usage: install-cli.sh [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --version TAG    Install a specific release (e.g. v0.9.0)"
      echo "  --source         Build from source via cargo install"
      echo "  --install-dir DIR  Installation directory (default: /usr/local/bin)"
      echo "  -h, --help       Show this help"
      echo ""
      echo "Environment:"
      echo "  INSTALL_DIR      Same as --install-dir"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

info()  { echo "  → $*"; }
error() { echo "ERROR: $*" >&2; exit 1; }

# --- Source install ---
if $FROM_SOURCE; then
  command -v cargo >/dev/null 2>&1 || error "cargo not found. Install Rust: https://rustup.rs"
  info "Building from source via cargo install..."

  CARGO_ROOT="$(mktemp -d)"
  trap 'rm -rf "$CARGO_ROOT"' EXIT

  if [[ -f "crates/cli/Cargo.toml" ]]; then
    cargo install --path crates/cli --root "$CARGO_ROOT" --force
  else
    cargo install --git "https://github.com/${REPO}" everruns-cli --root "$CARGO_ROOT" --force
  fi

  mkdir -p "$INSTALL_DIR"
  if [[ -w "$INSTALL_DIR" ]]; then
    mv "$CARGO_ROOT/bin/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
  else
    sudo mv "$CARGO_ROOT/bin/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
  fi

  info "Installed $("$INSTALL_DIR/$BINARY_NAME" --version) to $INSTALL_DIR/$BINARY_NAME"
  exit 0
fi

# --- Detect platform ---
detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin)
      case "$arch" in
        x86_64)  echo "x86_64-apple-darwin" ;;
        arm64)   echo "aarch64-apple-darwin" ;;
        *)       error "Unsupported macOS architecture: $arch" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64)  echo "x86_64-unknown-linux-gnu" ;;
        *)       error "Unsupported Linux architecture: $arch. Use --source to build from source." ;;
      esac
      ;;
    *)
      error "Unsupported OS: $os. Use --source to build from source."
      ;;
  esac
}

TARGET="$(detect_target)"
ARCHIVE="${BINARY_NAME}-${TARGET}.tar.gz"

# --- Resolve version ---
if [[ -z "$VERSION" ]]; then
  info "Fetching latest release..."
  VERSION="$(curl -fsSL "${CURL_OPTS[@]}" "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"
  [[ -n "$VERSION" ]] || error "Could not determine latest release version"
fi
info "Version: $VERSION"

# --- Download ---
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
CHECKSUM_URL="${DOWNLOAD_URL}.sha256"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

info "Downloading ${ARCHIVE}..."
curl -fSL "${CURL_OPTS[@]}" -o "$TMPDIR/$ARCHIVE" "$DOWNLOAD_URL" || error "Download failed. Check that release $VERSION has pre-built binaries for $TARGET."

info "Downloading checksum..."
if curl -fSL "${CURL_OPTS[@]}" -o "$TMPDIR/${ARCHIVE}.sha256" "$CHECKSUM_URL" 2>/dev/null; then
  info "Verifying checksum..."
  cd "$TMPDIR"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${ARCHIVE}.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${ARCHIVE}.sha256"
  else
    info "Warning: no sha256sum/shasum available, skipping verification"
  fi
  cd - >/dev/null
else
  info "Warning: checksum file not found, skipping verification"
fi

# --- Extract and install ---
info "Extracting..."
tar xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"

info "Installing to $INSTALL_DIR..."
if [[ -w "$INSTALL_DIR" ]] || [[ -w "$(dirname "$INSTALL_DIR")" ]]; then
  mkdir -p "$INSTALL_DIR"
  mv "$TMPDIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
  chmod +x "$INSTALL_DIR/$BINARY_NAME"
else
  sudo mkdir -p "$INSTALL_DIR"
  sudo mv "$TMPDIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
  sudo chmod +x "$INSTALL_DIR/$BINARY_NAME"
fi

info "Installed $("$INSTALL_DIR/$BINARY_NAME" --version) to $INSTALL_DIR/$BINARY_NAME"

# --- PATH check ---
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo ""
  echo "  Note: $INSTALL_DIR is not in your PATH."
  echo "  Add it with:  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
