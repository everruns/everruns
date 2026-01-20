#!/usr/bin/env bash
# Fast initialization for cloud agent environments (Claude Code on web, CI, etc.)
# Installs pre-built binaries instead of compiling from source.
#
# Usage: ./scripts/init-cloud-env.sh
#
# This script installs:
# - just: command runner (wraps dev.sh)
# - gh: GitHub CLI (for PR/issue operations)
#
# Run this BEFORE any other commands in a fresh cloud environment.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# Ensure ~/.cargo/bin exists and is in PATH
INSTALL_DIR="${HOME}/.cargo/bin"
mkdir -p "$INSTALL_DIR"
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    export PATH="$INSTALL_DIR:$PATH"
fi

install_just() {
    if command -v just &> /dev/null; then
        info "just already installed: $(just --version)"
        return 0
    fi

    info "Installing just (pre-built binary)..."

    # Use official installer script - downloads pre-built binary
    curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to "$INSTALL_DIR"

    if command -v just &> /dev/null; then
        info "just installed: $(just --version)"
    else
        error "Failed to install just"
    fi
}

install_gh() {
    if command -v gh &> /dev/null; then
        info "gh already installed: $(gh --version | head -1)"
        return 0
    fi

    info "Installing gh (GitHub CLI, pre-built binary)..."

    # Detect architecture
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64)  GH_ARCH="amd64" ;;
        aarch64) GH_ARCH="arm64" ;;
        armv7l)  GH_ARCH="armv6" ;;
        *)       error "Unsupported architecture: $ARCH" ;;
    esac

    # Get latest version from GitHub API
    GH_VERSION=$(curl -sS https://api.github.com/repos/cli/cli/releases/latest | grep '"tag_name"' | cut -d'"' -f4 | sed 's/^v//')

    if [[ -z "$GH_VERSION" ]]; then
        # Fallback version if API fails
        GH_VERSION="2.63.2"
        warn "Could not fetch latest gh version, using fallback: $GH_VERSION"
    fi

    GH_TARBALL="gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz"
    GH_URL="https://github.com/cli/cli/releases/download/v${GH_VERSION}/${GH_TARBALL}"

    # Download and extract
    TEMP_DIR=$(mktemp -d)
    trap "rm -rf $TEMP_DIR" EXIT

    info "Downloading gh v${GH_VERSION}..."
    curl -sSL "$GH_URL" -o "$TEMP_DIR/$GH_TARBALL"

    tar -xzf "$TEMP_DIR/$GH_TARBALL" -C "$TEMP_DIR"

    # Install binary
    cp "$TEMP_DIR/gh_${GH_VERSION}_linux_${GH_ARCH}/bin/gh" "$INSTALL_DIR/gh"
    chmod +x "$INSTALL_DIR/gh"

    if command -v gh &> /dev/null; then
        info "gh installed: $(gh --version | head -1)"
    else
        error "Failed to install gh"
    fi
}

main() {
    echo "================================================"
    echo "  Cloud Environment Initialization"
    echo "  Installing pre-built binaries for fast setup"
    echo "================================================"
    echo ""

    START_TIME=$(date +%s)

    install_just
    install_gh

    END_TIME=$(date +%s)
    ELAPSED=$((END_TIME - START_TIME))

    echo ""
    echo "================================================"
    info "Cloud environment ready in ${ELAPSED}s"
    echo ""
    echo "Installed tools:"
    echo "  - just $(just --version 2>/dev/null || echo '(not in PATH)')"
    echo "  - gh $(gh --version 2>/dev/null | head -1 || echo '(not in PATH)')"
    echo ""
    echo "Next steps:"
    echo "  just --list     # See available commands"
    echo "  just init       # Full dev environment setup"
    echo "  just start-dev  # Quick start (no Docker needed)"
    echo "================================================"
}

main "$@"
