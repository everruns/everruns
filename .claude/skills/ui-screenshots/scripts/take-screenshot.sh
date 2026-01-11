#!/usr/bin/env bash
set -euo pipefail

# Take a screenshot of a URL using agent-browser
#
# Usage: take-screenshot.sh <URL> <OUTPUT_PATH>
#
# Example:
#   ./take-screenshot.sh http://localhost:9100/dev/components screenshot.png
#
# Requires: agent-browser (npm install -g agent-browser && agent-browser install)
#
# Falls back to Playwright with sandbox-disabled flags in restricted environments.

URL="${1:-http://localhost:9100/dev/components}"
OUTPUT_PATH="${2:-screenshot.png}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

echo "📸 Taking screenshot of $URL"
echo "   Output: $OUTPUT_PATH"

# Create output directory if needed
OUTPUT_DIR=$(dirname "$OUTPUT_PATH")
if [ "$OUTPUT_DIR" != "." ] && [ ! -d "$OUTPUT_DIR" ]; then
  mkdir -p "$OUTPUT_DIR"
fi

# Convert to absolute path for Playwright fallback
if [[ "$OUTPUT_PATH" != /* ]]; then
  OUTPUT_PATH="$(pwd)/$OUTPUT_PATH"
fi

# Try agent-browser first
try_agent_browser() {
  if ! command -v agent-browser &> /dev/null; then
    return 1
  fi

  SESSION_NAME="screenshots"

  echo "   Using agent-browser..."
  if ! agent-browser --session "$SESSION_NAME" open "$URL" 2>/dev/null; then
    return 1
  fi

  sleep 2

  if ! agent-browser --session "$SESSION_NAME" screenshot "$OUTPUT_PATH" --full 2>/dev/null; then
    return 1
  fi

  return 0
}

# Fallback to Playwright with sandbox-disabled flags (for containers/restricted envs)
try_playwright_fallback() {
  echo "   Falling back to Playwright (sandbox-disabled)..."

  # Find chromium
  CHROMIUM_PATHS=(
    "/root/.cache/ms-playwright/chromium-1200/chrome-linux/chrome"
    "/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
  )

  CHROMIUM_PATH=""
  for path in "${CHROMIUM_PATHS[@]}"; do
    if [ -f "$path" ]; then
      CHROMIUM_PATH="$path"
      break
    fi
  done

  if [ -z "$CHROMIUM_PATH" ]; then
    echo "❌ Chromium not found"
    return 1
  fi

  cd "$PROJECT_ROOT/apps/ui"

  TEMP_SCRIPT=$(mktemp ./screenshot-XXXXXX.mjs)
  cat > "$TEMP_SCRIPT" << EOF
import { chromium } from 'playwright';

const browser = await chromium.launch({
  executablePath: '$CHROMIUM_PATH',
  args: [
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--disable-gpu',
    '--disable-software-rasterizer',
    '--disable-dev-shm-usage',
    '--single-process',
  ],
});

const page = await browser.newPage();

try {
  await page.goto('$URL', { waitUntil: 'networkidle', timeout: 30000 });
  await page.waitForTimeout(2000);
  await page.screenshot({ path: '$OUTPUT_PATH', fullPage: true });
} finally {
  await browser.close();
}
EOF

  node "$TEMP_SCRIPT"
  local result=$?
  rm -f "$TEMP_SCRIPT"
  return $result
}

# Try agent-browser, fall back to Playwright if it fails
if try_agent_browser; then
  echo "✅ Screenshot saved to $OUTPUT_PATH"
elif try_playwright_fallback; then
  echo "✅ Screenshot saved to $OUTPUT_PATH"
else
  echo "❌ Screenshot failed"
  exit 1
fi
