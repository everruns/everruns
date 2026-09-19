#!/usr/bin/env bash
# Record the terminal demo: a scripted worker editing a real repository while a
# real classifier watches it. The worker is deterministic so the recording is
# about the supervision; the nine numbers on screen are a live reading.
set -euo pipefail
demo_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$demo_dir/../../.." && pwd)"
tape="$demo_dir/demo.tape"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
gif_tmp="$demo_dir/.demo.tmp.gif"

extract_transcript() {
  sed $'s/\033\\[[0-?]*[ -\\/]*[@-~]//g' "$1" | awk '
    /everruns · foreman/ {
      capture = 1
      sub(/^.*everruns/, "everruns")
    }
    capture {
      gsub(/\r/, "")
      sub(/[[:space:]]+$/, "")
      print
    }
    /bash tests\/run.sh` passes/ { exit }
  '
}

if [[ "${1:-}" == "--check" ]]; then
  grep -Fq 'cargo run -q -p everruns-foreman-demo --bin foreman-demo -- --live-foreman' "$tape"
  grep -Fq 'Wait+Line@180s />$/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  sample="$(printf '\033[1meverruns · foreman\033[0m\r\n  \xe2\x9c\x93 `bash tests/run.sh` passes\r\n> ' | extract_transcript /dev/stdin)"
  [[ "$sample" == $'everruns · foreman\n  ✓ `bash tests/run.sh` passes' ]]
  if command -v vhs >/dev/null 2>&1; then
    vhs validate "$tape"
  fi
  echo "Foreman demo checks passed"
  exit 0
fi

command -v vhs >/dev/null 2>&1 || {
  echo "vhs is required to record the live demo" >&2
  exit 1
}

cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$gif_tmp"' EXIT
if [[ -n "${TYPESAFE_API_KEY:-}" ]]; then
  vhs "$tape"
else
  command -v doppler >/dev/null 2>&1 || {
    echo "set TYPESAFE_API_KEY or install Doppler to record the live demo" >&2
    exit 1
  }
  doppler run --project everruns-dev --config dev -- vhs "$tape"
fi

extract_transcript "$session_tmp" > "$transcript_tmp"
# Only publish a recording of a run that actually reached a decision.
grep -Fq 'FINISH' "$transcript_tmp"
grep -Fq 'status finished' "$transcript_tmp"
# The example's last word is a real test run, so the recording must reach it.
grep -Fq 'bash tests/run.sh` passes' "$transcript_tmp"
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"

echo "Recorded $demo_dir/demo.gif"
