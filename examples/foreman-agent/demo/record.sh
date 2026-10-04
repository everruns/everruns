#!/usr/bin/env bash
# Record the real binary; publish only a complete, successful supervised run.
set -euo pipefail
demo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$demo_dir/../../.." && pwd)"
tape="$demo_dir/demo.tape"
session_tmp="$demo_dir/.session.tmp"
transcript_tmp="$demo_dir/.transcript.tmp"
gif_tmp="$demo_dir/.demo.tmp.gif"
exit_tmp="$demo_dir/.exit.tmp"

extract_transcript() {
  sed $'s/\033\\[[0-?]*[ -\\/]*[@-~]//g' "$1" | awk '
    /FOREMAN · ready for a job|everruns · foreman/ { capture = 1 }
    capture { gsub(/\r/, ""); sub(/[[:space:]]+$/, ""); print }
    /✓ `bash tests\/run.sh` passes/ { exit }
  '
}

valid_take() {
  [[ -f "$2" && "$(cat "$2")" == 0 ]] || return 1
  local live
  live="$(grep -Ec 'SUPERVISOR · reading [0-9]+  observed worker-[0-9]+ RUNNING' "$1" || true)"
  [[ "$live" -ge 2 ]] || return 1
  # Check both successful markers and their order, not just their labels.
  awk '
    /FOREMAN · ready for a job/ { ready = 1 }
    ready && /shipkit · flat shipping rates, 3 existing tests/ { fixture = 1 }
    fixture && /request Replace flat rates/ { request = 1 }
    request && /→ START_WORKER / { dispatch = 1 }
    dispatch && /coding worker, attempt 1/ { coding = 1 }
    coding && /SUPERVISOR · reading [0-9]+  observed worker-[0-9]+ RUNNING/ { live = 1 }
    live && /→ START_VERIFIER / { verify = 1 }
    verify && /independent verifier · READ-ONLY workspace/ { readonly = 1 }
    readonly && /→ FINISH / { finish = 1 }
    finish && /status finished/ { finished = 1 }
    finished && /✓ fixed acceptance checks pass/ { acceptance = 1 }
    acceptance && /FINAL TEST RUN/ { suite = 1 }
    suite && /✓ `bash tests\/run.sh` passes/ { passed = 1 }
    END { exit !passed }
  ' "$1"
}

# Tests source the same publication gate without invoking VHS or providers.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then return; fi

if [[ "${1:-}" == "--check" ]]; then
  grep -Fq 'cargo run -q -p everruns-foreman-agent --bin foreman -- demo' "$tape"
  grep -Fq 'Wait+Line@180s />$/' "$tape"
  grep -Fq 'Type@35ms' "$tape"
  grep -Fq 'demo --interactive' "$tape"
  grep -Fq 'Wait+Line@30s /Job:$/' "$tape"
  bash "$repo_root/examples/foreman-agent/tests/recording.sh"
  if command -v vhs >/dev/null 2>&1; then vhs validate "$tape"; fi
  echo "Foreman demo checks passed"
  exit 0
fi

command -v vhs >/dev/null 2>&1 || { echo 'VHS is required' >&2; exit 1; }
cd "$repo_root"
trap 'rm -f "$session_tmp" "$transcript_tmp" "$gif_tmp" "$exit_tmp"' EXIT
cargo build -q -p everruns-foreman-agent --bin foreman
if [[ -n "${TYPESAFE_API_KEY:-}" && -n "${OPENROUTER_API_KEY:-}" ]]; then
  vhs "$tape"
else
  command -v doppler >/dev/null 2>&1 || { echo 'Export both provider keys or install Doppler' >&2; exit 1; }
  doppler run --project everruns-dev --config dev -- vhs "$tape"
fi

extract_transcript "$session_tmp" > "$transcript_tmp"
valid_take "$transcript_tmp" "$exit_tmp" || {
  echo 'Take rejected: need job entry, coding dispatch, live readings, read-only verification, FINISH, acceptance checks, final tests, and exit 0' >&2
  cat "$transcript_tmp" >&2
  exit 1
}
mv "$transcript_tmp" "$demo_dir/transcript.txt"
mv "$gif_tmp" "$demo_dir/demo.gif"
echo "Recorded $demo_dir/demo.gif"
