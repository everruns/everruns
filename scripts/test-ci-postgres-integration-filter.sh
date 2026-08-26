#!/usr/bin/env bash
# Guard the `postgres_integration` path filter against losing the crates that own
# provider wire behaviour.
#
# `Integration Tests (PostgreSQL)` is where the live provider matrix
# (`crates/llm-tests`) runs. Its gate is the `postgres_integration` filter in
# ci.yml. That filter used to enumerate drivers one by one — openai, anthropic,
# gemini — so every driver added afterwards (openrouter, bedrock, meta,
# fireworks, mai, llmsim) silently fell outside it, and `crates/provider` was
# never listed at all.
#
# The cost was not theoretical: the fix for the `main` regression in
# openresponses_protocol.rs (PR #3280) touched only `crates/provider`, so the
# live matrix that exists to validate wire serialization was skipped on the
# fix's own PR and the break surfaced on the merge commit instead (EVE-936).
#
# An enumeration cannot notice a crate that was never added to it, so the filter
# now uses directory-wide globs and this test pins that: every crate owning
# provider wire behaviour must be matched by some `postgres_integration` glob.
# Re-narrowing the filter to a hand-kept list turns this red instead of silently
# dropping a driver from live coverage.

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

# `filters` is a YAML document embedded as a block string in the step's `with`.
filters_raw = None
for step in jobs["changes"]["steps"]:
    if step.get("id") == "core_filter":
        filters_raw = step["with"]["filters"]
        break

if filters_raw is None:
    sys.exit(f"{workflow}: no `core_filter` paths-filter step found")

patterns = yaml.safe_load(filters_raw).get("postgres_integration")
if not patterns:
    sys.exit(f"{workflow}: `postgres_integration` filter is missing or empty")


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
required = [Path("crates/provider")]
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
        f"{workflow}: `postgres_integration` does not cover crates that own "
        "provider wire behaviour, so a change to them skips the live provider "
        "matrix (EVE-936): " + ", ".join(uncovered)
    )

print(f"postgres_integration covers all {len(required)} provider-owning crates")
PY
