#!/usr/bin/env bash
# Guard against CI reporting a pass for a run in which nothing executed.
#
# `Resolve CI event gate` sets `run_ci=false` for a `pull_request`
# `labeled`/`unlabeled` event carrying anything outside the `ci:skip-*` set, so
# every downstream job skips. That is the right call on its own — relabelling a
# PR should not rebuild it — but two things turned it into a false green
# (EVE-939):
#
#   1. The concurrency group was keyed only on the branch, so a label event
#      shared a group with the real run and cancelled it. Dependabot applies its
#      labels in the same second it opens the PR, so on #3284 six runs started
#      within one second and the survivor was the no-op labeled run.
#   2. `Build Check` failed only on "failure"/"cancelled". An all-skipped run
#      therefore reported success, and a required check said "pass" when the
#      honest reading was "nothing ran".
#
# All four open Dependabot PRs were mergeable and green having run no CI at all,
# including one bumping jsonschema and sha3 across Cargo.lock.
#
# Both halves are needed. (1) alone still loses to ordering: label a PR after a
# real run went red and the no-op run's newer green Build Check supersedes it.
# (2) alone lets the no-op keep cancelling real runs. This test pins both.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(dirname "$SCRIPT_DIR")"

python3 - <<'PY'
import sys
from pathlib import Path

import yaml

workflow = Path(".github/workflows/ci.yml")
doc = yaml.safe_load(workflow.read_text())

errors = []

# --- 1. label events must not share a concurrency group with code events -----
group = str(doc.get("concurrency", {}).get("group", ""))
mentions_action = "github.event.action" in group
distinguishes_label = "labeled" in group
if not (mentions_action and distinguishes_label):
    errors.append(
        f"{workflow}: concurrency.group does not separate label events from code "
        "events, so a labeled/unlabeled run can cancel the real run and become "
        "the surviving one (EVE-939). Current group:\n"
        f"  {group}"
    )

# --- 2. Build Check must refuse to pass a run where run_ci was false ---------
build_check = None
for job in doc["jobs"].values():
    if job.get("name") == "Build Check":
        build_check = job
        break

if build_check is None:
    sys.exit(f"{workflow}: no job named 'Build Check' found")

verify = "\n".join(
    step.get("run") or "" for step in build_check.get("steps") or []
)

# The gate must read run_ci and exit non-zero on it. Checking for both halves
# keeps this honest if the guard is ever reduced to a comment.
guards_run_ci = "run_ci" in verify and "exit 1" in verify
if not guards_run_ci:
    errors.append(
        f"{workflow}: the 'Build Check' verify step does not fail on "
        "run_ci != 'true'. A run where every job skipped would report success, "
        "turning 'nothing ran' into a green required check (EVE-939)."
    )

if errors:
    sys.exit("\n\n".join(errors))

print("ci.yml: label-event runs cannot cancel real runs, and Build Check "
      "refuses to pass a run that executed nothing")
PY
