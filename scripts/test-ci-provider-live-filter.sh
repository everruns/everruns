#!/usr/bin/env bash
# Paid provider coverage is a trusted nightly/full tier; ordinary PR/main stays offline.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PYCODE'
import re
from pathlib import Path
import yaml
workflow = Path(".github/workflows/ci.yml")
jobs = yaml.safe_load(workflow.read_text())["jobs"]
FILTER = "provider_live"
JOB = "live-provider-matrix"
job_if = jobs[JOB]["if"]
assert not jobs[JOB].get("services"), "provider wire tests need credentials, not a database"
assert JOB in jobs["ci-success"]["needs"], "provider failures must fail their nightly/full aggregate"
# Nightly/on-demand runs bypass change detection, but never the main-only
# credential boundary. Their binaries must build even without a Rust diff.
doc = yaml.safe_load(workflow.read_text())
triggers = doc.get("on", doc.get(True))
schedule = triggers.get("schedule", [])
assert len(schedule) == 1, "missing nightly live coverage"
assert schedule[0]["cron"].split()[2:] == ["*", "*", "*"], "sweep must run daily"
assert "workflow_dispatch" in triggers
assert "github.event_name == 'schedule'" in job_if
assert "github.ref == 'refs/heads/main'" in job_if
assert "outputs.provider_live" in jobs["build-binaries"]["if"]
assert "outputs.provider_live" in jobs["workflow-test"]["if"]

# Exercise the actual workflow expressions, including the empty filter outputs
# when schedules skip change detection. A PR must never reach either paid job.
def resolve(expression, values):
    expression = expression.removeprefix("${{").removesuffix("}}")
    expression = re.sub(
        r"\b(?:github|needs|steps)(?:\.[a-zA-Z_][a-zA-Z_0-9]*)+",
        lambda m: repr(values[m.group()]), expression,
    )
    return bool(eval(expression.replace("&&", " and ").replace("||", " or "), {"__builtins__": {}}))


for event, ref, changed, rust, paid_expected, workflow_expected in [
    ("push", "refs/heads/main", "true", "true", False, True),
    ("push", "refs/heads/main", "false", "true", False, True),
    ("push", "refs/heads/main", "false", "false", False, False),
    ("pull_request", "refs/pull/1/merge", "true", "true", False, False),
    ("schedule", "refs/heads/main", "", "", True, True),
    ("workflow_dispatch", "refs/heads/main", "", "", True, True),
    ("workflow_dispatch", "refs/heads/topic", "", "", False, False),
]:
    values = {
        "github.event_name": event, "github.ref": ref,
        "steps.core_filter.outputs.provider_live": changed,
        "steps.plan.outputs.mode": "pr" if event == "pull_request" else "nightly" if event == "schedule" else "full" if event == "workflow_dispatch" else "batch",
        "steps.plan.outputs.force": "true" if event in ("schedule", "workflow_dispatch") else "false",
        "needs.changes.outputs.mode": "pr" if event == "pull_request" else "nightly" if event == "schedule" else "full" if event == "workflow_dispatch" else "batch",
        "needs.changes.outputs.offline": "true" if event in ("schedule", "workflow_dispatch") else "false",
        "needs.changes.outputs.sdk_compat": "false",
        "needs.changes.outputs.docker": "false",
        "needs.changes.outputs.run_ci": "true",
        "needs.changes.outputs.rust": "true" if event in ("schedule", "workflow_dispatch") else rust,
        "needs.changes.outputs.budget_e2e": "",
        "needs.changes.outputs.skip_slow_rust": "false",
    }
    paid = resolve(jobs["changes"]["outputs"][FILTER], values)
    values["needs.changes.outputs.provider_live"] = str(paid).lower()
    assert resolve(job_if, values) == paid_expected, (event, ref)
    assert resolve(jobs["workflow-test"]["if"], values) == workflow_expected, (event, ref)
    if workflow_expected:
        assert resolve(jobs["build-binaries"]["if"], values), (event, ref)

# Ordinary Rust pushes retain llmsim coverage without fetching provider keys.
steps = jobs["workflow-test"]["steps"]
paid = next(s for s in steps if s.get("name") == "Run workflow tests")
assert "outputs.provider_live == 'true'" in paid["if"]
sim = next(s for s in steps if s.get("name") == "Run llmsim workflow tests")
assert "outputs.provider_live != 'true'" in sim["if"]
assert "DOPPLER_TOKEN" not in sim.get("env", {})
assert "doppler" not in sim["run"]
for test in [
    "test_no_duplicate_tool_calls", "test_agent_execution_openai_with_tool_calls",
    "test_agent_execution_anthropic_with_tool_calls",
    "test_agent_filesystem_and_bash_workspace_integration",
    "test_anthropic_extended_thinking", "test_anthropic_extended_thinking_with_tools",
    "test_reasoning_reaches_api_sanitized_and_classified",
]:
    assert f"--skip {test}" in sim["run"], test

print("provider coverage: nightly/full on trusted main; ordinary PR/main stays unpaid")
PYCODE
