#!/usr/bin/env bash
# Executable contracts for focused PRs, coalesced main, release gates and triage.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 .github/scripts/ci/test_policy.py
