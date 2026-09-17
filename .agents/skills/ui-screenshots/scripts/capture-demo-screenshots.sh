#!/usr/bin/env bash
set -euo pipefail

# Capture the maintained Everruns demo screenshot set in matching light/dark pairs.
#
# Usage: capture-demo-screenshots.sh [BASE_URL] [OUTPUT_DIR]
# Example: capture-demo-screenshots.sh http://localhost:27100 assets/screenshots
#
# Playwright renders a 1440x900 CSS viewport at DPR 2, producing 2880x1800 PNGs without changing
# page composition. The required demo state and review contract live in:
# knowledge/ui/demo-screenshots.md

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

if ! command -v node >/dev/null 2>&1; then
  echo "node not found; install the repository's UI dependencies before capturing screenshots." >&2
  exit 1
fi

node "${SCRIPT_DIR}/capture-demo-screenshots.mjs" "$@"
