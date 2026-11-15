#!/usr/bin/env bash
# Installs the Everruns sandbox guest agent at /usr/local/bin, where every
# Isolated sandbox pod starts it (kubernetes_sandbox::GUEST_BINARY).
#
# Decision: the binary is static (musl), so the feature works on any Linux
# base image, Debian, Ubuntu, Alpine or Fedora alike. It is downloaded from
# the Everruns GitHub release and checked against the release's SHA-256.
set -euo pipefail

VERSION="${VERSION:-latest}"
REPO="https://github.com/everruns/everruns/releases"
DEST=/usr/local/bin/everruns-sandbox-guest

case "$(uname -m)" in
  x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
  aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
  *) echo "everruns sandbox guest: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

if [ "$VERSION" = latest ]; then
  base="$REPO/latest/download"
else
  case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
  base="$REPO/download/$VERSION"
fi

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    echo "everruns sandbox guest: needs curl or wget" >&2
    exit 1
  fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
archive="everruns-sandbox-guest-$target.tar.gz"
fetch "$base/$archive" "$work/$archive"
fetch "$base/$archive.sha256" "$work/$archive.sha256"
(cd "$work" && sha256sum -c "$archive.sha256" >/dev/null)
tar -C "$work" -xzf "$work/$archive" everruns-sandbox-guest
install -m 0755 "$work/everruns-sandbox-guest" "$DEST"
mkdir -p /workspace
"$DEST" --version
