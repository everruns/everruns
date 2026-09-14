#!/usr/bin/env bash
set -euo pipefail

# Take a screenshot of a URL using agent-browser
#
# Usage: take-screenshot.sh <URL> <OUTPUT_PATH>
#
# Example:
#   ./take-screenshot.sh http://localhost:9300/dev/components screenshot.png
#
# Requires: agent-browser >= 0.8.5 (npm install -g agent-browser && agent-browser install)
#
# Supports containerized and cloud-agent environments via AGENT_BROWSER_ARGS.

URL="${1:-http://localhost:9300/dev/components}"
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

# Chrome flags, passed through AGENT_BROWSER_ARGS (newline-separated) rather
# than `--args` (comma-separated). Two reasons, both found the hard way in
# EVE-807: `--args` splits on commas, so a multi-value flag such as
# `--disable-features=A,B` is parsed as two argv entries and Chrome exits with
# "Multiple targets are not supported in headless mode"; and flags passed only
# on the first `open` are silently dropped when the daemon relaunches, which
# looks like an intermittent failure rather than a missing flag.
#
# `--ssl-version-max=tls1.2` is the one that matters outside a container.
# Chrome's TLS 1.3 ClientHello carries a post-quantum key share that pushes it
# to roughly 2 KB; the cloud-agent egress relay accepts the CONNECT and then
# cuts the tunnel mid-handshake, which reaches Chrome as a bare
# ERR_CONNECTION_RESET on *every* HTTPS page. `curl` and `openssl s_client`
# through the same proxy are unaffected because neither offers a PQ key share —
# which is why the proxy looks healthy when tested by hand. Capping the version
# the client offers shrinks the ClientHello below the relay's breaking point.
# Harmless outside the sandbox, so it is unconditional.
BROWSER_ARGS=$'--ssl-version-max=tls1.2\n--disable-features=PostQuantumKyber'
EXTRA_OPTS=""

# Running as root or in a container also needs the sandbox disabled.
if [ "$(id -u)" = "0" ] || [ -f /.dockerenv ]; then
  BROWSER_ARGS+=$'\n--no-sandbox\n--disable-setuid-sandbox\n--disable-dev-shm-usage\n--disable-gpu\n--single-process'

  # Find a Chromium build. PLAYWRIGHT_BROWSERS_PATH is set in cloud-agent
  # containers, where the browser is not under ~/.cache/ms-playwright.
  for path in \
    "${PLAYWRIGHT_BROWSERS_PATH:-/opt/pw-browsers}"/chromium-*/chrome-linux/chrome \
    "${PLAYWRIGHT_BROWSERS_PATH:-/opt/pw-browsers}"/chromium-*/chrome-linux64/chrome \
    /root/.cache/ms-playwright/chromium-*/chrome-linux/chrome \
    /root/.cache/ms-playwright/chromium-*/chrome-linux64/chrome; do
    if [ -f "$path" ]; then
      EXTRA_OPTS="--executable-path $path"
      break
    fi
  done
fi

export AGENT_BROWSER_ARGS="$BROWSER_ARGS"

# Close any existing session to apply new launch options
agent-browser --session "$SESSION_NAME" close 2>/dev/null || true
sleep 1

# Navigate to the URL
echo "   Opening page..."
if [ -n "$EXTRA_OPTS" ]; then
  # shellcheck disable=SC2086 # EXTRA_OPTS is a flag pair built above, not user input
  agent-browser --session "$SESSION_NAME" $EXTRA_OPTS open "$URL"
else
  agent-browser --session "$SESSION_NAME" open "$URL"
fi

# A relaunched daemon drops flags, and the symptom (ERR_CONNECTION_RESET on
# HTTPS) looks nothing like "a flag went missing". Say so while it is cheap.
if ! pgrep -a chrome 2>/dev/null | grep -q -- '--ssl-version-max'; then
  echo "   ⚠️  Chromium is running without --ssl-version-max=tls1.2;" >&2
  echo "      HTTPS pages will fail with ERR_CONNECTION_RESET behind the agent proxy (EVE-807)." >&2
fi

# Wait for page to stabilize
echo "   Waiting for page load..."
sleep 2

# Take full-page screenshot
echo "   Capturing screenshot..."
agent-browser --session "$SESSION_NAME" screenshot "$OUTPUT_PATH" --full

echo "✅ Screenshot saved to $OUTPUT_PATH"
