#!/usr/bin/env bash
set -euo pipefail
demo_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$demo_dir/../../.." && pwd)"
tape="$demo_dir/demo.tape"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
gif_tmp="$demo_dir/.demo.tmp.gif"

extract_transcript() {
  sed $'s/\033\\[[0-?]*[ -\\/]*[@-~]//g' "$1" | awk '
    /everruns · code mode over a huge MCP server/ {
      capture = 1
      sub(/^.*everruns/, "everruns")
    }
    capture {
      gsub(/\r/, "")
      sub(/[[:space:]]+$/, "")
      print
    }
    /Completed: answer verified against the MCP server/ { exit }
  '
}

if [[ "${1:-}" == "--check" ]]; then
  grep -Fq 'cargo run -q -p everruns-code-mode-mcp-agent -- --interactive' "$tape"
  grep -Fq 'Wait+Screen /Question:/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  grep -Fq 'Wait+Line@240s />$/' "$tape"
  sample="$(printf '\033[1meverruns · code mode over a huge MCP server\033[0m\r\nCompleted: answer verified against the MCP server\r\n> ' | extract_transcript /dev/stdin)"
  [[ "$sample" == $'everruns · code mode over a huge MCP server\nCompleted: answer verified against the MCP server' ]]
  if command -v vhs >/dev/null 2>&1; then
    vhs validate "$tape"
  fi
  echo "Code Mode MCP Agent demo checks passed"
  exit 0
fi

command -v vhs >/dev/null 2>&1 || {
  echo "vhs is required to record the live demo" >&2
  exit 1
}

cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$gif_tmp"' EXIT
if [[ -n "${OPENROUTER_API_KEY:-}" || -n "${OPENAI_API_KEY:-}" ]]; then
  vhs "$tape"
else
  command -v doppler >/dev/null 2>&1 || {
    echo "set OPENROUTER_API_KEY or OPENAI_API_KEY, or install Doppler, to record the live demo" >&2
    exit 1
  }
  doppler run --project everruns-dev --config dev -- vhs "$tape"
fi

extract_transcript "$session_tmp" > "$transcript_tmp"
grep -Fq 'Completed: answer verified against the MCP server' "$transcript_tmp"
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"

echo "Recorded $demo_dir/demo.gif"
