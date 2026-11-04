#!/usr/bin/env bash
# Hosted Ubuntu ships versioned LLVM tools; the lld metapackage only adds aliases.
set -euo pipefail

# Newest versioned ld.lld-N on PATH. The runner image moves LLVM majors (18 on
# 24.04, 21 on 26.04); pinning one sent every job on the newer image to apt,
# where a packages.microsoft.com 403 failed full run 39757685091. Pure bash:
# the step must not depend on sort -V or other tools.
newest_versioned_lld() {
  local candidate best="" best_major=-1 major
  while IFS= read -r candidate; do
    major="${candidate#ld.lld-}"
    [[ "$major" =~ ^[0-9]+$ ]] || continue
    if (( major > best_major )); then
      best_major="$major"
      best="$candidate"
    fi
  done < <(compgen -c ld.lld- || true)
  [ -n "$best" ] && command -v "$best"
}

if command -v ld.lld >/dev/null 2>&1; then
  ld.lld --version
elif installed="$(newest_versioned_lld)"; then
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
