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
for step in jobs["changes"]["steps"]:
    if step.get("id") == "live_filter":
        filters_raw = step["with"]["filters"]
        break

if filters_raw is None:
    sys.exit(f"{workflow}: no `live_filter` paths-filter step found")

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
        "command": "cargo test -p everruns-integrations-daytona",
        "probes": [
            "integrations/daytona/src/session_sandbox_provider.rs",
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
        "command": "cargo test -p everruns-integrations-e2b",
        "probes": [
            "integrations/e2b/src/client.rs",
            ".github/workflows/ci.yml",
        ],
    },
    "browserless": {
        "job": "browserless-live-test",
        "command": "cargo test -p everruns-integrations-browserless",
        "probes": [
            "integrations/browserless/src/lib.rs",
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

for provider, expected in providers.items():
    patterns = filters[provider]
    for probe in [
        "integrations/daytona/README.md",
        "integrations/e2b/SPEC.md",
        "integrations/browserless/README.md",
        "apps/ui/src/app/page.tsx",
    ]:
        assert not included(patterns, probe), (provider, probe)

print("sandbox live filters: shared Daytona contract and Doppler wiring covered")
PY
