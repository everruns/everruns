#!/usr/bin/env bash
# sccache helper: install binary + configure S3 backend via Doppler.
# Optional — builds work without it, just slower on cold cache.
#
# Usage:
#   source scripts/lib/sccache.sh   # sets RUSTC_WRAPPER + env vars
#   ./scripts/lib/sccache.sh setup  # install + configure (idempotent)
#   ./scripts/lib/sccache.sh stats  # show cache stats
#   ./scripts/lib/sccache.sh stop   # stop server

set -euo pipefail

SCCACHE_FALLBACK_VERSION="v0.14.0"
INSTALL_DIR="${HOME}/.cargo/bin"

# --- helpers ---------------------------------------------------------------

_info()  { echo -e "\033[0;32m[sccache]\033[0m $1"; }
_warn()  { echo -e "\033[1;33m[sccache]\033[0m $1"; }
_error() { echo -e "\033[0;31m[sccache]\033[0m $1"; }

_install_sccache() {
  if command -v sccache &>/dev/null; then
    _info "already installed: $(sccache --version)"
    return 0
  fi

  _info "Installing sccache (pre-built binary)..."

  local arch
  arch=$(uname -m)
  case "$arch" in
    x86_64)  arch="x86_64" ;;
    aarch64) arch="aarch64" ;;
    arm64)   arch="aarch64" ;;  # macOS
    *) _error "Unsupported architecture: $arch"; return 1 ;;
  esac

  local os target
  case "$(uname -s)" in
    Linux)  os="unknown-linux-musl" ;;
    Darwin) os="apple-darwin" ;;
    *) _error "Unsupported OS: $(uname -s)"; return 1 ;;
  esac
  target="${arch}-${os}"

  # Pinned version — skip GitHub API call to avoid rate limits and hangs
  local version="$SCCACHE_FALLBACK_VERSION"

  local tarball="sccache-${version}-${target}.tar.gz"
  local url="https://github.com/mozilla/sccache/releases/download/${version}/${tarball}"

  local tmp
  tmp=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" EXIT

  _info "Downloading sccache ${version} (${target})..."
  curl -fsSL --connect-timeout 10 --max-time 60 --retry 2 --retry-delay 2 "$url" -o "$tmp/$tarball"
  tar -xzf "$tmp/$tarball" -C "$tmp"

  mkdir -p "$INSTALL_DIR"
  cp "$tmp/sccache-${version}-${target}/sccache" "$INSTALL_DIR/sccache"
  chmod +x "$INSTALL_DIR/sccache"

  if command -v sccache &>/dev/null; then
    _info "Installed: $(sccache --version)"
  else
    _warn "Installed to $INSTALL_DIR/sccache but not in PATH"
    _warn "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
  fi
}

# Export env vars for sccache S3 backend. Requires Doppler.
# No-op if Doppler unavailable or secrets missing.
_configure_s3() {
  if ! command -v doppler &>/dev/null; then
    _warn "doppler not found — using sccache with local storage only"
    return 0
  fi

  # Check if sccache S3 secrets exist in Doppler
  local bucket
  bucket=$(doppler secrets get SCCACHE_BUCKET --plain 2>/dev/null) || true
  if [[ -z "$bucket" ]]; then
    _warn "SCCACHE_BUCKET not in Doppler — using sccache with local storage only"
    return 0
  fi

  # Map Doppler secrets to env vars sccache expects
  export SCCACHE_BUCKET="$bucket"
  export SCCACHE_REGION="$(doppler secrets get SCCACHE_REGION --plain 2>/dev/null || echo 'us-east-1')"
  export SCCACHE_S3_KEY_PREFIX="$(doppler secrets get SCCACHE_S3_KEY_PREFIX --plain 2>/dev/null || echo 'sccache')"
  export AWS_ACCESS_KEY_ID="$(doppler secrets get SCCACHE_AWS_ACCESS_KEY_ID --plain 2>/dev/null)"
  export AWS_SECRET_ACCESS_KEY="$(doppler secrets get SCCACHE_AWS_SECRET_ACCESS_KEY --plain 2>/dev/null)"

  _info "S3 backend: s3://${SCCACHE_BUCKET}/${SCCACHE_S3_KEY_PREFIX}"
}

# Activate sccache: set RUSTC_WRAPPER. Safe to call multiple times.
# Returns 1 if sccache not available (caller can continue without it).
activate_sccache() {
  if ! command -v sccache &>/dev/null; then
    return 1
  fi

  _configure_s3
  export RUSTC_WRAPPER="$(command -v sccache)"
  export CARGO_INCREMENTAL=0  # required for sccache
  _info "Activated: RUSTC_WRAPPER=$(command -v sccache)"
}

# --- CLI -------------------------------------------------------------------

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  cmd="${1:-setup}"
  case "$cmd" in
    setup)
      _install_sccache
      activate_sccache || _warn "sccache installed but activation failed"
      ;;
    install)
      _install_sccache
      ;;
    activate)
      activate_sccache || { _error "sccache not installed. Run: $0 setup"; exit 1; }
      ;;
    stats)
      sccache --show-stats
      ;;
    stop)
      sccache --stop-server 2>/dev/null && _info "Server stopped" || _info "Server not running"
      ;;
    *)
      echo "Usage: $0 {setup|install|activate|stats|stop}"
      exit 1
      ;;
  esac
fi
