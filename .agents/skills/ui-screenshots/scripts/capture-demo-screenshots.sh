#!/usr/bin/env bash
set -euo pipefail

# Capture the maintained Everruns demo screenshot set in matching light/dark pairs.
#
# Usage: capture-demo-screenshots.sh [BASE_URL] [OUTPUT_DIR]
# Example: capture-demo-screenshots.sh http://localhost:27100 assets/screenshots
#
# The required demo state and review contract live in:
# knowledge/ui/demo-screenshots.md

BASE_URL="${1:-http://localhost:27100}"
BASE_URL="${BASE_URL%/}"
OUTPUT_DIR="${2:-assets/screenshots}"
SESSION_NAME="everruns-demo-screenshots"

if ! command -v agent-browser >/dev/null 2>&1; then
  echo "agent-browser not found; install it with: npm install -g agent-browser" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq not found; install it before capturing demo screenshots." >&2
  exit 1
fi

if ! curl -fsS "${BASE_URL}/health" >/dev/null; then
  echo "Everruns is not healthy at ${BASE_URL}; start the local stack first." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

platform_chat_id=$(curl -fsS "${BASE_URL}/api/v1/sessions" | jq -er '
  [.data[] | select(.title == "Platform Chat" and .source == "chat" and .is_pinned == true)]
  | first
  | .id
')

cleanup() {
  agent-browser --session "$SESSION_NAME" close >/dev/null 2>&1 || true
}
trap cleanup EXIT

agent-browser --session "$SESSION_NAME" close >/dev/null 2>&1 || true
agent-browser --session "$SESSION_NAME" open "$BASE_URL"
agent-browser --session "$SESSION_NAME" set viewport 1440 900

scenes=(
  "platform-chat:/chats/${platform_chat_id}"
  "sessions:/sessions"
  "agents:/agents"
  "harnesses:/harnesses"
  "durable-execution:/durable"
)

for theme in light dark; do
  agent-browser --session "$SESSION_NAME" set media "$theme"

  for scene in "${scenes[@]}"; do
    name="${scene%%:*}"
    route="${scene#*:}"
    output="${OUTPUT_DIR}/${name}-${theme}.png"

    echo "Capturing ${name} (${theme})"
    agent-browser --session "$SESSION_NAME" open "${BASE_URL}${route}"
    agent-browser --session "$SESSION_NAME" wait --load networkidle
    sleep 0.3

    # A revisited chat can restore its previous nested scroll position. Reset every scroll container
    # so light and dark variants frame the same state instead of drifting between captures.
    agent-browser --session "$SESSION_NAME" eval \
      "window.scrollTo(0, 0); document.querySelectorAll('*').forEach((element) => { if (element.scrollHeight > element.clientHeight) element.scrollTop = 0; })" \
      >/dev/null

    # The local Next.js development portal is browser tooling, not product UI. Removing it keeps
    # presentation assets clean; these images must never be used as debugging or test evidence.
    agent-browser --session "$SESSION_NAME" eval \
      "document.querySelectorAll('nextjs-portal').forEach((element) => element.remove())" \
      >/dev/null
    agent-browser --session "$SESSION_NAME" screenshot "$output"
  done
done

echo "Captured ${#scenes[@]} scenes in light and dark themes under ${OUTPUT_DIR}."
