#!/usr/bin/env bash
# Fast initialization for cloud agent environments (Claude Code on web, CI, etc.)
# Installs pre-built binaries instead of compiling from source.
#
# Usage: ./scripts/init-cloud-env.sh
#
# This script installs:
# - just: command runner (see justfile)
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

configure_gh_repo() {
    # Set default repo for gh CLI (needed when git remote uses local proxy)
    # Extract repo from git remote URL (handles both github.com and proxy URLs)
    local remote_url repo

    remote_url=$(git remote get-url origin 2>/dev/null || echo "")
    if [[ -z "$remote_url" ]]; then
        warn "No git remote found, skipping gh repo configuration"
        return 0
    fi

    # Extract owner/repo from URL patterns:
    # - https://github.com/owner/repo.git
    # - git@github.com:owner/repo.git
    # - http://proxy@127.0.0.1:PORT/git/owner/repo
    if [[ "$remote_url" =~ github\.com[:/]([^/]+/[^/.]+) ]]; then
        repo="${BASH_REMATCH[1]}"
    elif [[ "$remote_url" =~ /git/([^/]+/[^/.]+) ]]; then
        repo="${BASH_REMATCH[1]}"
    else
        warn "Could not extract repo from remote URL: $remote_url"
        return 0
    fi

    # Remove .git suffix if present
    repo="${repo%.git}"

    # Check current default
    local current_default
    current_default=$(gh repo set-default --view 2>/dev/null || echo "")

    if [[ "$current_default" == *"$repo"* ]]; then
        info "gh default repo already set: $repo"
        return 0
    fi

    # gh repo set-default requires a remote pointing to GitHub
    # Add a 'github' remote if origin uses a proxy
    if [[ ! "$remote_url" =~ github\.com ]]; then
        local github_url="https://github.com/${repo}.git"
        if ! git remote get-url github &>/dev/null; then
            info "Adding 'github' remote: $github_url"
            git remote add github "$github_url"
        fi
        # Fetch main branch so github/main ref exists (needed for gh pr merge)
        # Always fetch in case remote exists but was never fetched
        if ! git rev-parse --verify github/main &>/dev/null; then
            info "Fetching main branch from github remote..."
            git fetch github main 2>/dev/null || warn "Failed to fetch github/main"
        fi
        # Use the github remote for set-default
        gh repo set-default github 2>/dev/null && info "gh default repo set: $repo" || warn "Failed to set default repo"
    else
        gh repo set-default "$repo" 2>/dev/null && info "gh default repo set: $repo" || warn "Failed to set default repo"
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
    configure_gh_repo

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
    echo "  just --list              # See available commands"
    echo "  just start-dev --no-watch  # Quick start (recommended for cloud)"
    echo "  just init                # Full dev environment setup (for local dev)"
    echo "================================================"
}

main "$@"
