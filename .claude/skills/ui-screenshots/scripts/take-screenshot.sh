#!/usr/bin/env bash
set -euo pipefail

# Take a screenshot of a URL using agent-browser
#
# Usage: take-screenshot.sh <URL> <OUTPUT_PATH>
#
# Example:
#   ./take-screenshot.sh http://localhost:9100/dev/components screenshot.png
#
# Requires: agent-browser >= 0.8.5 (npm install -g agent-browser && agent-browser install)
#
# Supports containerized environments via --args flag for sandbox-disabling.

URL="${1:-http://localhost:9100/dev/components}"
OUTPUT_PATH="${2:-screenshot.png}"

# Check if agent-browser is installed
if ! command -v agent-browser &> /dev/null; then
  echo "❌ agent-browser not found. Install with:"
  echo "   npm install -g agent-browser"
  echo "   agent-browser install"
  exit 1
fi

echo "📸 Taking screenshot of $URL"
echo "   Output: $OUTPUT_PATH"

# Create output directory if needed
OUTPUT_DIR=$(dirname "$OUTPUT_PATH")
if [ "$OUTPUT_DIR" != "." ] && [ ! -d "$OUTPUT_DIR" ]; then
  mkdir -p "$OUTPUT_DIR"
fi

# Convert to absolute path
if [[ "$OUTPUT_PATH" != /* ]]; then
  OUTPUT_PATH="$(pwd)/$OUTPUT_PATH"
fi

# Use a dedicated session for screenshots
SESSION_NAME="screenshots"

# Build launch args for containerized/sandboxed environments
LAUNCH_ARGS=""
EXTRA_OPTS=""

# Detect if running as root or in container (needs --no-sandbox)
if [ "$(id -u)" = "0" ] || [ -f /.dockerenv ]; then
  LAUNCH_ARGS="--no-sandbox,--disable-setuid-sandbox,--disable-dev-shm-usage,--disable-gpu,--single-process"

  # Find chromium executable
  CHROMIUM_PATHS=(
    "/root/.cache/ms-playwright/chromium-1200/chrome-linux64/chrome"
    "/root/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
    "/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
  )

  for path in "${CHROMIUM_PATHS[@]}"; do
    if [ -f "$path" ]; then
      EXTRA_OPTS="--executable-path $path --args \"$LAUNCH_ARGS\""
      break
    fi
  done
fi

# Close any existing session to apply new launch options
agent-browser --session "$SESSION_NAME" close 2>/dev/null || true
sleep 1

# Navigate to the URL
echo "   Opening page..."
if [ -n "$EXTRA_OPTS" ]; then
  eval "agent-browser --session \"$SESSION_NAME\" $EXTRA_OPTS open \"$URL\""
else
  agent-browser --session "$SESSION_NAME" open "$URL"
fi

# Wait for page to stabilize
echo "   Waiting for page load..."
sleep 2

# Take full-page screenshot
echo "   Capturing screenshot..."
agent-browser --session "$SESSION_NAME" screenshot "$OUTPUT_PATH" --full

echo "✅ Screenshot saved to $OUTPUT_PATH"
