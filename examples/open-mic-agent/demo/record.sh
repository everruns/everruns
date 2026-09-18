#!/usr/bin/env bash
set -euo pipefail
demo_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$demo_dir/../../.." && pwd)"
tape="$demo_dir/demo.tape"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
gif_tmp="$demo_dir/.demo.tmp.gif"

extract_transcript() {
  awk '
    /MODEL: / { capture = 1; sub(/^.*MODEL:/, "MODEL:") }
    capture { gsub(/\r/, ""); print }
    /Completed:/ { exit }
  ' "$1"
}

if [[ "${1:-}" == "--check" ]]; then
  grep -Fq 'cargo run -q -p everruns-open-mic-agent -- --interactive' "$tape"
  grep -Fq 'Wait+Screen /Submission:/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  grep -Fq 'Wait@3m' "$tape"
  sample="$(printf '\033[0mMODEL: test\r\nANSWER\r\nCompleted: test\r\n> ' | extract_transcript /dev/stdin)"
  [[ "$sample" == $'MODEL: test\nANSWER\nCompleted: test' ]]
  if command -v vhs >/dev/null 2>&1; then
    vhs validate "$tape"
  fi
  echo "Open Mic Agent demo checks passed"
  exit 0
fi

command -v vhs >/dev/null 2>&1 || {
  echo "vhs is required to record the live demo" >&2
  exit 1
}

cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$gif_tmp"' EXIT
# The agent needs both credentials: OpenAI answers, TypeSafe measures.
if [[ -n "${ANTHROPIC_API_KEY:-}" && -n "${TYPESAFE_API_KEY:-}" ]]; then
  vhs "$tape"
else
  command -v doppler >/dev/null 2>&1 || {
    echo "set ANTHROPIC_API_KEY and TYPESAFE_API_KEY, or install Doppler, to record the live demo" >&2
    exit 1
  }
  doppler run --project everruns-dev --config dev -- vhs "$tape"
fi

extract_transcript "$session_tmp" > "$transcript_tmp"
grep -Fq 'Completed:' "$transcript_tmp"
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"

echo "Recorded $demo_dir/demo.gif"
