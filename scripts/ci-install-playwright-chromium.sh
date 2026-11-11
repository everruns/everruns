#!/usr/bin/env bash
# Install Playwright's Chromium for CI, reaching apt only when a library is
# actually missing.
#
# Decision (2026-11-11): `playwright install --with-deps chromium` always runs
# apt, so when the runners could not resolve the Ubuntu archive the UI jobs
# failed after a ten-minute stall even though the runner image already carries
# every library Chromium links against. Download the browser first, ask the
# dynamic linker what is missing, and run `install-deps` only for a real gap.
#
# Run from the package directory that owns the Playwright dependency.

set -euo pipefail

pnpm exec playwright install chromium

browsers="${PLAYWRIGHT_BROWSERS_PATH:-$HOME/.cache/ms-playwright}"
missing=""
while IFS= read -r -d '' binary; do
  gaps="$(ldd "$binary" 2>/dev/null | grep 'not found' || true)"
  if [ -n "$gaps" ]; then
    missing+="${binary}:"$'\n'"${gaps}"$'\n'
  fi
done < <(find "$browsers" -type f \( -name chrome -o -name headless_shell \) -print0 2>/dev/null)

if [ -z "$missing" ]; then
  echo "Chromium system libraries already present; skipping apt"
  exit 0
fi

echo "Chromium is missing system libraries; installing them with apt:"
printf '%s' "$missing"
pnpm exec playwright install-deps chromium
