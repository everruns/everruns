#!/usr/bin/env bash
# Keep space for cache extraction, Rust artifacts and the linker's temporary output.
set -euo pipefail

directories=(/usr/share/dotnet /usr/local/lib/android /opt/ghc)
case "${1:-}" in
  '') ;;
  --extended)
    directories+=(/opt/hostedtoolcache/CodeQL /usr/local/share/powershell
      /usr/local/share/chromium /usr/local/lib/node_modules)
    ;;
  *) echo 'usage: ci-prepare-disk.sh [--extended]' >&2; exit 1 ;;
esac

available_kib="$(LC_ALL=C df -Pk . | awk 'NR == 2 { print $4 }')"
if [[ ! "$available_kib" =~ ^[0-9]+$ ]]; then
  echo 'Cannot determine available build disk space' >&2
  exit 1
fi
df -h .
# Hosted images can already have >80 GiB free. Deleting Android there costs
# over a minute without helping the build. Smaller images retain prior cleanup.
if (( available_kib >= 50 * 1024 * 1024 )); then
  echo 'Skipping SDK cleanup: at least 50 GiB already available'
else
  for directory in "${directories[@]}"; do
    start=$SECONDS
    sudo rm -rf "$directory"
    echo "Cleanup $directory: $((SECONDS - start))s"
    df -h .
  done
fi
