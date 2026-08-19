#!/usr/bin/env bash
# Guard ci.yml against inheriting the 360-minute GitHub Actions default timeout.
#
# A job with no `timeout-minutes` does not fail when a package mirror stalls — it
# squats for six hours, and because ci.yml jobs are required checks, every open PR
# sits at mergeable_state=blocked for that whole window with no signal telling
# "installing" apart from "wedged" (EVE-921). An explicit ceiling turns a wedge
# into a legible red in minutes.
#
# Two rules:
#   1. every job binds a timeout-minutes, at or below CEILING_MINUTES;
#   2. every step that shells out to apt binds its own, so the failure names the
#      install rather than the whole job.

set -euo pipefail

python3 - <<'PY'
from pathlib import Path

import yaml

# Generous relative to observed runtimes (the slowest job sampled over ~120 recent
# runs finishes in under 7 minutes). A job that trips this is wedged, not slow.
CEILING_MINUTES = 45

# Commands that reach a package mirror and can hang rather than fail.
APT_MARKERS = ("apt-get", "apt install", "--with-deps")

path = Path(".github/workflows/ci.yml")
jobs = yaml.safe_load(path.read_text())["jobs"]

errors = []

missing = [name for name, job in jobs.items() if "timeout-minutes" not in job]
if missing:
    errors.append(
        f"{path}: jobs without timeout-minutes inherit the 360m default: "
        + ", ".join(sorted(missing))
    )

excessive = [
    f"{name} ({job['timeout-minutes']}m)"
    for name, job in jobs.items()
    if isinstance(job.get("timeout-minutes"), int)
    and job["timeout-minutes"] > CEILING_MINUTES
]
if excessive:
    errors.append(
        f"{path}: timeout-minutes above the {CEILING_MINUTES}m ceiling: "
        + ", ".join(sorted(excessive))
    )

unbounded_installs = []
for name, job in jobs.items():
    for step in job.get("steps") or []:
        run = step.get("run") or ""
        if any(marker in run for marker in APT_MARKERS):
            if "timeout-minutes" not in step:
                label = step.get("name") or run.strip().splitlines()[0]
                unbounded_installs.append(f"{name}: {label}")
if unbounded_installs:
    errors.append(
        f"{path}: package-install steps without a step-level timeout-minutes: "
        + ", ".join(sorted(unbounded_installs))
    )

if errors:
    raise SystemExit("\n".join(errors))

print(
    f"all {len(jobs)} ci.yml jobs bound a timeout-minutes at or below "
    f"{CEILING_MINUTES}m, and every package-install step bounds its own"
)
PY
