#!/usr/bin/env bash
# The recorded session: type the documented command, then run it for real.
# The command must match the one in demo.tape; record.sh --check enforces that.
set -euo pipefail
cd "$(dirname "$0")/../../.."

command='cargo run -q -p everruns-spam-triage -- --limit 30'
printf '\033[1;35m$\033[0m '
for (( index = 0; index < ${#command}; index++ )); do
  printf '%s' "${command:index:1}"
  sleep 0.035
done
printf '\n'
eval "$command"
printf '\n\033[1;35m$\033[0m '
sleep 3
