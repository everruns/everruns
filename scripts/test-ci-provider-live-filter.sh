#!/usr/bin/env bash
# Guard the `provider_live` path filter against losing the crates that own
# provider wire behaviour.
#
# The `Live Provider Matrix` job (`crates/llm-tests`) is the only live
# validation of what actually goes on the wire to a provider. Its gate is the
# `provider_live` filter in ci.yml. That gate used to enumerate drivers one by
# one — openai, anthropic, gemini — so every driver added afterwards
# (openrouter, bedrock, meta, fireworks, mai, llmsim) silently fell outside it,
# and `crates/provider` was never listed at all.
#
# The cost was not theoretical: the fix for the `main` regression in
# openresponses_protocol.rs (PR #3280) touched only `crates/provider`, so the
# live matrix that exists to validate wire serialization was skipped on the
# fix's own PR and the break surfaced on the merge commit instead (EVE-936).
#
# An enumeration cannot notice a crate that was never added to it, so the filter
# covers every driver's source and this test pins that: every crate owning provider
# wire behaviour must be matched by some `provider_live` glob. Re-narrowing the
# filter to a hand-kept list turns this red instead of silently dropping a
# driver from live coverage.
#
# It also pins the wiring itself. Until EVE-938 the matrix ran inside
# `Integration Tests (PostgreSQL)` and was gated by a filter shaped around
# server persistence — the filter and the thing it gated had drifted apart, and
# nothing noticed. Checking the globs alone would not catch that recurring: the
# job's `if:` has to keep reading `provider_live`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(dirname "$SCRIPT_DIR")"

python3 - <<'PY'
import re
import sys
from pathlib import Path

import yaml

workflow = Path(".github/workflows/ci.yml")
jobs = yaml.safe_load(workflow.read_text())["jobs"]

FILTER = "provider_live"
JOB = "live-provider-matrix"

# `filters` is a YAML document embedded as a block string in the step's `with`.
filters_raw = None
for step in jobs["changes"]["steps"]:
    if step.get("id") == "core_filter":
        filters_raw = step["with"]["filters"]
        break

if filters_raw is None:
    sys.exit(f"{workflow}: no `core_filter` paths-filter step found")

patterns = yaml.safe_load(filters_raw).get(FILTER)
if not patterns:
    sys.exit(f"{workflow}: `{FILTER}` filter is missing or empty")

if JOB not in jobs:
    sys.exit(f"{workflow}: no `{JOB}` job found — the live provider matrix lost its job")

# The filter only guards anything while the job it gates actually consults it.
job_if = str(jobs[JOB].get("if", ""))
if f"outputs.{FILTER}" not in job_if:
    sys.exit(
        f"{workflow}: `{JOB}` does not gate on `needs.changes.outputs.{FILTER}`, so "
        f"the `{FILTER}` filter no longer decides whether the live provider matrix "
        "runs (EVE-938)"
    )

# The matrix has no database and must not reacquire one: a PostgreSQL service
# is what coupled its gate to server persistence in the first place (EVE-938).
if jobs[JOB].get("services"):
    sys.exit(
        f"{workflow}: `{JOB}` declares services; the live provider matrix needs "
        "provider credentials, not infrastructure (EVE-938)"
    )

# Build Check is the branch-protection aggregate. Keep the standalone live job
# in its dependency set so a provider failure cannot produce a green aggregate.
aggregate_needs = jobs.get("ci-success", {}).get("needs", [])
if JOB not in aggregate_needs:
    sys.exit(
        f"{workflow}: `ci-success` does not need `{JOB}`, so Build Check cannot "
        "report failures from the live provider matrix"
    )


def matches(pattern: str, path: str) -> bool:
    """Approximate micromatch: `**` crosses directories, `*` does not."""
    regex = ""
    i = 0
    while i < len(pattern):
        if pattern.startswith("**", i):
            regex += ".*"
            i += 2
        elif pattern[i] == "*":
            regex += "[^/]*"
            i += 1
        else:
            regex += re.escape(pattern[i])
            i += 1
    return re.fullmatch(regex, path) is not None


# Crates whose source decides what goes on the wire to a provider. `provider`
# owns the shared protocols; each `drivers/*` owns one provider's binding.
# `llm-tests` owns the matrix itself.
required = [Path("crates/contracts"), Path("crates/provider"), Path("crates/llm-tests")]
required += sorted(p for p in Path("crates/drivers").iterdir() if p.is_dir())

uncovered = []
for crate in required:
    if not (crate / "Cargo.toml").is_file():
        continue
    # A representative source path: coverage of the crate means coverage of the
    # files that can change its wire behaviour.
    probe = f"{crate.as_posix()}/src/lib.rs"
    if not any(matches(pattern, probe) for pattern in patterns):
        uncovered.append(crate.as_posix())

if uncovered:
    sys.exit(
        f"{workflow}: `{FILTER}` does not cover crates that own provider wire "
        "behaviour, so a change to them skips the live provider matrix "
        "(EVE-936): " + ", ".join(uncovered)
    )

# Unrelated workspace/deployment changes must not buy a full live matrix.
for probe in [
    "Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".github/workflows/ci.yml",
    "crates/drivers/drivers/Cargo.toml", "crates/provider/README.md",
    "crates/server/src/api/sessions.rs", "apps/ui/src/app/page.tsx",
]:
    assert not any(matches(pattern, probe) for pattern in patterns), probe

for probe in [
    "crates/contracts/src/model_profile_data/profiles/gpt6.rs",
    "crates/server/src/seed/models.rs",
    "crates/server/src/platform.rs",
]:
    assert any(matches(pattern, probe) for pattern in patterns), probe

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
    ("push", "refs/heads/main", "true", "true", True, True),
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
        "needs.changes.outputs.run_ci": "true",
        "needs.changes.outputs.rust": rust,
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

print(f"{FILTER}: provider/model changes and nightly coverage; ordinary CI stays unpaid")
PY
