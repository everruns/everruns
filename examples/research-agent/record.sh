#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

# Preserve the previous successful transcript if the provider call fails.
task_transcript=$(mktemp)
trap 'rm -f "$task_transcript"' EXIT
cargo run -q -p everruns-research-agent -- "$@" | tee "$task_transcript"
mv "$task_transcript" demo.txt
vhs demo.tape
