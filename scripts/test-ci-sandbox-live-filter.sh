#!/usr/bin/env bash
# Pin live sandbox-provider routing and credential scope in ci.yml.

set -euo pipefail

cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
from pathlib import Path

import yaml

workflow = Path(".github/workflows/ci.yml")
jobs = yaml.safe_load(workflow.read_text())["jobs"]

filters_raw = None
predicate_quantifier = None
for step in jobs["changes"]["steps"]:
    if step.get("id") == "live_filter":
        filters_raw = step["with"]["filters"]
        predicate_quantifier = step["with"].get("predicate-quantifier")
        break

if filters_raw is None:
    sys.exit(f"{workflow}: no `live_filter` paths-filter step found")

if predicate_quantifier != "some-with-excludes":
    sys.exit(
        f"{workflow}: `live_filter` must use `some-with-excludes` so one changed "
        "file can match any positive provider/shared path while negated docs stay excluded"
    )

filters = yaml.safe_load(filters_raw)


def matches(pattern: str, path: str) -> bool:
    """Approximate micromatch: `**` crosses directories, `*` does not."""
    regex = ""
    i = 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            regex += "(?:.*/)?"
            i += 3
        elif pattern.startswith("**", i):
            regex += ".*"
            i += 2
        elif pattern[i] == "*":
            regex += "[^/]*"
            i += 1
        else:
            regex += re.escape(pattern[i])
            i += 1
    return re.fullmatch(regex, path) is not None


def included(patterns: list[str], path: str) -> bool:
    positive = [pattern for pattern in patterns if not pattern.startswith("!")]
    negative = [pattern[1:] for pattern in patterns if pattern.startswith("!")]
    return any(matches(pattern, path) for pattern in positive) and not any(
        matches(pattern, path) for pattern in negative
    )


providers = {
    "daytona": {
        "job": "daytona-live-test",
        "command": "cargo test -p everruns-integrations --features daytona-live-tests --test daytona_live_api_test",
        "probes": [
            "crates/integrations/src/daytona/session_sandbox_provider.rs",
            "crates/contracts/src/session_sandbox.rs",
            "crates/contracts/src/sandbox_checkpoint.rs",
            "crates/capabilities/src/session_sandbox.rs",
            "crates/capabilities/src/session_sandbox/sandbox_recovery.rs",
            "crates/capabilities/src/sandbox_checkpoint.rs",
            "crates/capabilities/src/sandbox_state.rs",
            ".github/workflows/ci.yml",
        ],
    },
    "e2b": {
        "job": "e2b-live-test",
        "command": "cargo test -p everruns-integrations --features e2b-live-tests --test e2b_live_api_test",
        "probes": [
            "crates/integrations/src/e2b/client.rs",
            ".github/workflows/ci.yml",
        ],
    },
}

aggregate_needs = jobs.get("ci-success", {}).get("needs", [])

for provider, expected in providers.items():
    patterns = filters.get(provider)
    if not patterns:
        sys.exit(f"{workflow}: `{provider}` live filter is missing or empty")

    for probe in expected["probes"]:
        if not included(patterns, probe):
            sys.exit(
                f"{workflow}: `{provider}` live filter does not cover `{probe}`, "
                "so the Doppler-backed test can skip its own integration surface"
            )

    job_name = expected["job"]
    job = jobs.get(job_name)
    if job is None:
        sys.exit(f"{workflow}: no `{job_name}` job found")

    job_if = str(job.get("if", ""))
    for gate in [
        f"needs.changes.outputs.{provider} == 'true'",
        "github.event_name == 'push'",
        "github.ref == 'refs/heads/main'",
    ]:
        if gate not in job_if:
            sys.exit(f"{workflow}: `{job_name}` lost trusted live-test gate `{gate}`")

    live_steps = [
        step for step in job.get("steps", [])
        if isinstance(step.get("run"), str) and "doppler run --" in step["run"]
    ]
    if len(live_steps) != 1:
        sys.exit(f"{workflow}: `{job_name}` must have exactly one `doppler run --` step")
    live_step = live_steps[0]
    if live_step.get("env", {}).get("DOPPLER_TOKEN") != "${{ secrets.DOPPLER_TOKEN }}":
        sys.exit(f"{workflow}: `{job_name}` must scope DOPPLER_TOKEN to its live run step")
    if expected["command"] not in live_step["run"]:
        sys.exit(f"{workflow}: `{job_name}` no longer runs the expected live test")

    if job_name not in aggregate_needs:
        sys.exit(f"{workflow}: Build Check does not depend on `{job_name}`")

sweep = yaml.safe_load(Path(".github/workflows/integration-live-sweep.yml").read_text())
sweep_tests = {
    entry["name"]: entry["command"]
    for entry in sweep["jobs"]["doppler-backed-live-tests"]["strategy"]["matrix"]["include"]
}
for name, command in {
    "Daytona Live API Tests": providers["daytona"]["command"],
    "E2B Live API Tests": providers["e2b"]["command"],
    "Cursor Live API Tests": "cargo test -p everruns-integrations --features cursor-live-tests --test cursor_live_api_test",
}.items():
    if not sweep_tests.get(name, "").startswith(command):
        sys.exit(f"integration live sweep: `{name}` must run `{command}`")

# Browserless bills per browser session on a small credit allowance: its live
# suite runs only from browserless-integration.yml, on pushes to main that touch
# the integration or on manual dispatch. Never from ci.yml or the weekly sweep.
browserless_command = "cargo test -p everruns-integrations --features browserless-live-tests --test browserless_live_api"
if "browserless" in filters or "browserless-live-test" in jobs:
    sys.exit(f"{workflow}: Browserless live tests must not run from ci.yml")
if any("browserless" in name.lower() for name in sweep_tests):
    sys.exit("integration live sweep: Browserless must stay out of the weekly sweep")
browserless_path = Path(".github/workflows/browserless-integration.yml")
browserless = yaml.safe_load(browserless_path.read_text())
# PyYAML reads the bare `on` key as boolean True.
triggers = browserless.get("on", browserless.get(True))
if set(triggers) != {"workflow_dispatch", "push"}:
    sys.exit(f"{browserless_path}: must trigger only on push and workflow_dispatch, got {sorted(triggers)}")
push = triggers["push"]
if push.get("branches") != ["main"]:
    sys.exit(f"{browserless_path}: push trigger must be limited to main")
for probe in [
    "crates/integrations/src/browserless/mod.rs",
    "crates/integrations/tests/browserless_live_api.rs",
]:
    if not included(push["paths"], probe):
        sys.exit(f"{browserless_path}: push paths do not cover `{probe}`")
for probe in [
    "crates/integrations/src/browserless/README.md",
    ".github/workflows/ci.yml",
    "crates/integrations/src/daytona/mod.rs",
    "Cargo.lock",
]:
    if included(push["paths"], probe):
        sys.exit(f"{browserless_path}: push paths must not cover `{probe}`")
browserless_live_runs = [
    step
    for job in browserless["jobs"].values()
    for step in job.get("steps") or []
    if "doppler run --" in str(step.get("run", ""))
]
if len(browserless_live_runs) != 1 or browserless_command not in browserless_live_runs[0]["run"]:
    sys.exit(f"{browserless_path}: must have exactly one `doppler run --` step running the live suite")
if browserless_live_runs[0].get("env", {}).get("DOPPLER_TOKEN") != "${{ secrets.DOPPLER_TOKEN }}":
    sys.exit(f"{browserless_path}: must scope DOPPLER_TOKEN to its live run step")
if any(job.get("if") != "github.ref == 'refs/heads/main'" for job in browserless["jobs"].values()):
    sys.exit(f"{browserless_path}: live job must stay gated to main")

cursor = yaml.safe_load(Path(".github/workflows/cursor-integration.yml").read_text())
cursor_live_runs = [
    step.get("run", "")
    for job in cursor["jobs"].values()
    for step in job.get("steps") or []
    if "DOPPLER_TOKEN" in (step.get("env") or {})
]
if not any("--test cursor_live_api_test" in run for run in cursor_live_runs):
    sys.exit("cursor integration workflow must target `cursor_live_api_test`")

for provider, expected in providers.items():
    patterns = filters[provider]
    for probe in [
        "crates/integrations/src/daytona/README.md",
        "crates/integrations/src/e2b/SPEC.md",
        "crates/integrations/src/browserless/README.md",
        "apps/ui/src/app/page.tsx",
    ]:
        assert not included(patterns, probe), (provider, probe)

print("sandbox live filters: shared Daytona contract, Doppler wiring, and Browserless credit gate covered")
PY
