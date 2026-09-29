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
# It also pins the opt-out policy for UI jobs because those labels use the same
# event gate and must not suppress an affected full-stack budget test.

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

# --- 2b. ...but it must fail closed by deferring, not by blanket refusal -----
# Refusing unconditionally is the other half of the trap: GitHub resolves a
# required check by the newest run of that name, so a label applied after CI
# finished supersedes the real green and blocks a PR nobody touched
# (EVE-1077). The gated-off run must look for a verdict a real run already
# published on the same head SHA, and inherit only a pass.
defers_to_head_sha = (
    "check-runs" in verify
    and "check_name=Build+Check" in verify
    and "HEAD_SHA" in verify
)
if not defers_to_head_sha:
    errors.append(
        f"{workflow}: the 'Build Check' verify step does not consult the Checks "
        "API for a 'Build Check' that already passed on the pull request head "
        "SHA. Without that, a label event landing after CI finished turns a "
        "green PR red and blocks it (EVE-1077)."
    )

# Inheriting anything other than a pass would let a gated-off run launder a
# real failure into a green required check.
if 'select(.conclusion == "success")' not in verify:
    errors.append(
        f"{workflow}: the 'Build Check' verify step does not restrict the "
        "verdict it inherits to a successful run, so a gated-off run could "
        "report a pass the real run never gave (EVE-939)."
    )

# Reading those check runs needs the scope; without it the query 404s and the
# deferral silently becomes the blanket refusal it replaced.
permissions = doc.get("permissions") or {}
if permissions.get("checks") != "read":
    errors.append(
        f"{workflow}: workflow permissions do not grant 'checks: read', so "
        "'Build Check' cannot read the verdict it is meant to defer to "
        "(EVE-1077). Current permissions:\n"
        f"  {permissions}"
    )
# --- 3. the UI opt-out must guard every UI E2E path --------------------------
opt_out_policy = None
for job in doc["jobs"].values():
    if job.get("name") == "CI Opt-Out Policy":
        opt_out_policy = job
        break

if opt_out_policy is None:
    errors.append(f"{workflow}: no job named 'CI Opt-Out Policy' found")
else:
    policy_script = "\n".join(
        step.get("run") or "" for step in opt_out_policy.get("steps") or []
    )
    ui_guard_lines = [
        line
        for line in policy_script.splitlines()
        if 'require_if_affected "ci:skip-ui-e2e"' in line
    ]
    for affected_output, check_name in [
        ("needs.changes.outputs.ui", "UI Playwright smoke"),
        ("needs.changes.outputs.budget_e2e", "UI endpoint budget full-stack E2E"),
    ]:
        if not any(
            affected_output in line and check_name in line for line in ui_guard_lines
        ):
            errors.append(
                f"{workflow}: ci:skip-ui-e2e is not blocked when "
                f"{affected_output} is true; it can suppress affected {check_name} coverage."
            )

if errors:
    sys.exit("\n\n".join(errors))

print("ci.yml: label-event runs cannot cancel real runs, Build Check refuses "
      "to pass a run that executed nothing unless a real run already passed on "
      "the same head SHA, and UI E2E opt-outs cannot suppress affected smoke "
      "or endpoint-budget coverage")
PY
