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

URL="${1:-http://localhost:9100/dev/components}"
OUTPUT_PATH="${2:-screenshot.png}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check if agent-browser is installed
if ! command -v agent-browser &> /dev/null; then
  echo "❌ agent-browser not found. Install with:"
  echo "   npm install -g agent-browser"
  echo "   agent-browser install"
  echo ""
  echo "   Or run: $SCRIPT_DIR/install-browser.sh"
  exit 1
fi

echo "📸 Taking screenshot of $URL"
echo "   Output: $OUTPUT_PATH"

# Create output directory if needed
OUTPUT_DIR=$(dirname "$OUTPUT_PATH")
if [ "$OUTPUT_DIR" != "." ] && [ ! -d "$OUTPUT_DIR" ]; then
  mkdir -p "$OUTPUT_DIR"
fi

# Use a dedicated session for screenshots to avoid conflicts
SESSION_NAME="screenshots"

# Navigate to the URL
echo "   Opening page..."
agent-browser --session "$SESSION_NAME" open "$URL"

# Wait for page to stabilize (network idle equivalent)
echo "   Waiting for page load..."
sleep 2

# Take full-page screenshot
echo "   Capturing screenshot..."
agent-browser --session "$SESSION_NAME" screenshot "$OUTPUT_PATH" --full

echo "✅ Screenshot saved to $OUTPUT_PATH"
