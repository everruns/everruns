#!/usr/bin/env bash
# Install ripgrep for CI guard steps without depending on the Ubuntu mirror.
#
# Decision (2026-11-11): GitHub-hosted runners could not resolve
# archive.ubuntu.com / security.ubuntu.com for over half an hour, and every
# guard that ran `apt-get install ripgrep` turned main red although no code had
# changed. The guards only need the `rg` binary, so prefer, in order:
#   1. the `rg` already on PATH (runner image or a previous step);
#   2. the pinned upstream release binary, verified by SHA-256;
#   3. apt, only on architectures without a pinned binary.
#
# Overrides for tests: RG_INSTALL_DIR (default /usr/local/bin) and
# RG_BASE_URL (default the upstream GitHub release URL; file:// works).

set -euo pipefail

if command -v rg >/dev/null 2>&1; then
  rg --version | head -n 1
  exit 0
fi

RG_VERSION="14.1.1"
case "$(uname -m)" in
  x86_64)
    RG_TARGET="x86_64-unknown-linux-musl"
    RG_SHA256="4cf9f2741e6c465ffdb7c26f38056a59e2a2544b51f7cc128ef28337eeae4d8e"
    ;;
  aarch64 | arm64)
    RG_TARGET="aarch64-unknown-linux-gnu"
    RG_SHA256="c827481c4ff4ea10c9dc7a4022c8de5db34a5737cb74484d62eb94a95841ab2f"
    ;;
  *)
    echo "no pinned ripgrep for $(uname -m); falling back to apt" >&2
    sudo apt-get update || true
    sudo apt-get install -y ripgrep
    rg --version | head -n 1
    exit 0
    ;;
esac

RG_BASE_URL="${RG_BASE_URL:-https://github.com/BurntSushi/ripgrep/releases/download/${RG_VERSION}}"
RG_INSTALL_DIR="${RG_INSTALL_DIR:-/usr/local/bin}"
RG_DIR="ripgrep-${RG_VERSION}-${RG_TARGET}"
RG_TARBALL="${RG_DIR}.tar.gz"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# GitHub release downloads 504 intermittently; a fixed delay rides out a short
# outage (same policy as the Caddy download in ci.yml).
curl -fsSL --retry 8 --retry-delay 15 "${RG_BASE_URL}/${RG_TARBALL}" -o "${work}/${RG_TARBALL}"
echo "${RG_SHA256}  ${work}/${RG_TARBALL}" | sha256sum -c --quiet -
tar -xzf "${work}/${RG_TARBALL}" -C "$work" "${RG_DIR}/rg"

if [ -w "$RG_INSTALL_DIR" ]; then
  install -m 0755 "${work}/${RG_DIR}/rg" "${RG_INSTALL_DIR}/rg"
else
  sudo install -m 0755 "${work}/${RG_DIR}/rg" "${RG_INSTALL_DIR}/rg"
fi

"${RG_INSTALL_DIR}/rg" --version | head -n 1
