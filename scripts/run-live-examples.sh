#!/usr/bin/env bash
# Run the Framework examples that talk to a real model and fail on any that
# does not finish cleanly.
#
# Compile checks do not catch an example that builds but fails at run time:
# `hello` shipped for a while failing `Agent::build()` with DuplicateCapability
# because its tool name collided with a built-in capability. Examples run
# against a real LLM (AGENTS.md), so this needs OPENAI_API_KEY; run it through
# `doppler run --` locally, or from the weekly Integration Live Sweep.
#
# Usage: scripts/run-live-examples.sh [example ...]
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "run-live-examples: OPENAI_API_KEY is not set" >&2
  exit 1
fi

# name:required-features. Keep to examples that drive a real model and finish
# on their own; interactive or server examples belong elsewhere.
DEFAULT_EXAMPLES=(
  hello:openai
  production_agent:openai
  observe_and_cancel:openai
  subagents:openai
  lifecycle_hooks:macros,openai
  advanced_capability:capabilities,openai
  lua_code_mode_agent:lua,openai
  real_disk_file_system_tools:filesystem,openai
  in_process_runtime:openai
)

selected=("$@")
if [[ ${#selected[@]} -eq 0 ]]; then
  selected=("${DEFAULT_EXAMPLES[@]}")
fi

per_example_timeout="${LIVE_EXAMPLE_TIMEOUT:-300}"
failed=()
for entry in "${selected[@]}"; do
  name="${entry%%:*}"
  features=""
  [[ "$entry" == *:* ]] && features="${entry#*:}"
  echo "::group::example ${name}"
  # Build outside the timeout so a cold compile does not count against the run.
  cargo build -q -p everruns ${features:+--features "$features"} --example "$name"
  if timeout "$per_example_timeout" \
    cargo run -q -p everruns ${features:+--features "$features"} --example "$name" </dev/null; then
    echo "ok: ${name}"
  else
    echo "FAILED: ${name} (exit $?)"
    failed+=("$name")
  fi
  echo "::endgroup::"
done

if [[ ${#failed[@]} -gt 0 ]]; then
  echo "run-live-examples: failed: ${failed[*]}" >&2
  exit 1
fi
echo "run-live-examples: ${#selected[@]} example(s) passed"
