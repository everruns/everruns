#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/../demo/record.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
cat > "$scratch/good" <<'TRANSCRIPT'
FOREMAN · ready for a job
  repository shipkit · flat shipping rates, 3 existing tests
Job:
     request Replace flat rates with these tiers, add boundary tests, update README, and run the suite.
everruns · foreman
  → START_WORKER  dispatching the coding role
▸ worker-1 coding worker, attempt 1
  SUPERVISOR · reading 1  observed worker-1 RUNNING · 0s · 0 tools
  SUPERVISOR · reading 2  observed worker-1 RUNNING · 6s · 1 tools
  → START_VERIFIER  independent verification is warranted
▸ worker-2 independent verifier · READ-ONLY workspace, attempt 1
  → FINISH  completion thresholds satisfied
      status finished
  ✓ fixed acceptance checks pass
FINAL TEST RUN · bash tests/run.sh
  9 passed, 0 failed
  ✓ `bash tests/run.sh` passes
TRANSCRIPT
printf 0 > "$scratch/exit"
valid_take "$scratch/good" "$scratch/exit"
for marker in 'ready for a job' '3 existing tests' 'request Replace' 'START_WORKER' 'coding worker' 'START_VERIFIER' 'READ-ONLY' 'FINISH ' 'status finished' 'fixed acceptance' 'FINAL TEST RUN' 'RUNNING'; do
  sed "/$marker/d" "$scratch/good" > "$scratch/bad"
  if valid_take "$scratch/bad" "$scratch/exit"; then echo "Accepted missing $marker" >&2; exit 1; fi
done
sed 's/✓/✗/g' "$scratch/good" > "$scratch/bad"
if valid_take "$scratch/bad" "$scratch/exit"; then echo 'Accepted failing checks' >&2; exit 1; fi
printf 1 > "$scratch/exit"
if valid_take "$scratch/good" "$scratch/exit"; then echo 'Accepted nonzero exit' >&2; exit 1; fi
printf '\033[1meverruns · foreman\033[0m\r\n  ✓ `bash tests/run.sh` passes\r\n> ' > "$scratch/ansi"
[[ "$(extract_transcript "$scratch/ansi")" == $'everruns · foreman\n  ✓ `bash tests/run.sh` passes' ]]
echo 'Recording publication gate tests passed'
