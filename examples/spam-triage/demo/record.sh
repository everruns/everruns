#!/usr/bin/env bash
set -euo pipefail
demo_dir="$(cd "$(dirname "$0")" && pwd)"
example_dir="$(cd "$demo_dir/.." && pwd)"
repo_root="$(cd "$example_dir/.." && pwd)/.."
repo_root="$(cd "$repo_root" && pwd)"
tape="$demo_dir/demo.tape"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
gif_tmp="$demo_dir/.demo.tmp.gif"

extract_transcript() {
  awk '
    /SCREEN / { capture = 1; sub(/^.*SCREEN /, "SCREEN ") }
    capture { gsub(/\r/, ""); print }
    /adjudication / { exit }
  ' "$1"
}

if [[ "${1:-}" == "--check" ]]; then
  grep -Fq 'cargo run -q -p everruns-spam-triage -- --limit 30' "$tape"
  grep -Fq 'Wait+Screen /screening/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  grep -Fq 'Wait+Screen /SUMMARY/' "$tape"
  sample="$(printf '\033[0mSCREEN jev\r\nSUMMARY\r\n  adjudication 1.00s\r\n> ' | extract_transcript /dev/stdin)"
  [[ "$sample" == $'SCREEN jev\nSUMMARY\n  adjudication 1.00s' ]]
  if command -v vhs >/dev/null 2>&1; then
    vhs validate "$tape"
  fi
  echo "Spam Triage demo checks passed"
  exit 0
fi

[[ -f "$example_dir/data/corpus.jsonl" ]] || {
  echo "build the corpus first: bash examples/spam-triage/data/build-corpus.sh" >&2
  exit 1
}

cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$gif_tmp"' EXIT
if [[ -n "${TYPESAFE_API_KEY:-}" && -n "${OPENROUTER_API_KEY:-}" ]]; then
  vhs "$tape"
else
  command -v doppler >/dev/null 2>&1 || {
    echo "set TYPESAFE_API_KEY and OPENROUTER_API_KEY or install Doppler to record the live demo" >&2
    exit 1
  }
  doppler run --project everruns-dev --config dev -- vhs "$tape"
fi

extract_transcript "$session_tmp" > "$transcript_tmp"
grep -Fq 'adjudication' "$transcript_tmp"
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"

echo "Recorded $demo_dir/demo.gif"
