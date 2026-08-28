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
# uses directory-wide globs and this test pins that: every crate owning provider
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
required = [Path("crates/provider"), Path("crates/llm-tests")]
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

print(f"{FILTER} covers all {len(required)} provider-owning crates and gates {JOB}")
PY
