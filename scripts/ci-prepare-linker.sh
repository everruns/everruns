#!/usr/bin/env bash
# Hosted Ubuntu ships versioned LLVM tools; the lld metapackage only adds aliases.
set -euo pipefail

if command -v ld.lld >/dev/null 2>&1; then
  ld.lld --version
elif installed="$(command -v ld.lld-18)"; then
  # Keep the standard -fuse-ld=lld spelling and cache fingerprints. The alias
  # lives only in this runner's temporary directory, without sudo or apt I/O.
  directory="${RUNNER_TEMP:?}/ci-linker"
  mkdir -p "$directory"
  ln -sf "$installed" "$directory/ld.lld"
  printf '%s\n' "$directory" >> "${GITHUB_PATH:?}"
  "$directory/ld.lld" --version
else
  sudo apt-get update -qq
  sudo apt-get install -y -qq lld
  ld.lld --version
fi
