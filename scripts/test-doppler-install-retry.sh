#!/usr/bin/env bash
# Guard every `dopplerhq/cli-action` install against a single transient failure.
#
# The installer fetches the release tarball and its signature from the GitHub
# release CDN. That CDN is not ours and it does fail: on Integration Live Sweep
# run 34131004174 the `Brave Search API Tests` job died with
#
#     ERROR: Signature download failed with status code 504. Please try again.
#
# 32 seconds in, at step 3 of 7 — before a single test body ran. The seven other
# jobs in the same run installed the same CLI in 1-2 seconds. One unlucky job
# turned the whole scheduled sweep red while proving nothing about the code.
#
# A tool install that dies fetching bytes from someone else's CDN is not a
# result, so it should not be reported as one. Each install therefore gets one
# guarded retry, and this test keeps that property from eroding as workflows are
# added or edited.
#
# The guard is deliberately narrow, because "retry until green" is how a real
# failure gets laundered into a flake:
#
#   1. exactly one retry, not a loop;
#   2. the retry is NOT continue-on-error, so a second failure still fails the
#      job — this widens no fail-open window;
#   3. the retry fires only on `outcome == 'failure'`, never on a skip.
#
# Rule 3 is load-bearing for TM-CI-001/TM-CI-002. `ci.yml`'s live provider
# matrix installs Doppler only `if: github.event_name == 'push'`, keeping
# DOPPLER_TOKEN away from runners executing PR-controlled `cargo test`. A
# skipped step reports `outcome == 'skipped'`, not `'failure'`, so the retry
# cannot resurrect that install on a pull request. Matching on the outcome
# alone — rather than restating the push gate on the retry — also keeps the
# gate in exactly one place, where it cannot drift out of sync.

set -euo pipefail

python3 - <<'PY'
from pathlib import Path

import yaml

ACTION = "dopplerhq/cli-action@"
RETRY_IF = "steps.doppler_install.outcome == 'failure'"

errors = []
guarded = 0

for path in sorted(Path(".github/workflows").glob("*.yml")):
    document = yaml.safe_load(path.read_text())
    if not isinstance(document, dict):
        continue

    for job_name, job in (document.get("jobs") or {}).items():
        steps = job.get("steps") or []
        where = f"{path}:{job_name}"

        installs = [
            (index, step)
            for index, step in enumerate(steps)
            if isinstance(step.get("uses"), str) and step["uses"].startswith(ACTION)
        ]
        if not installs:
            continue

        # A guarded pair is (first attempt, retry). Anything else is unguarded.
        first = [(i, s) for i, s in installs if s.get("id") == "doppler_install"]
        retries = [(i, s) for i, s in installs if s.get("if") == RETRY_IF]

        if len(first) != 1 or len(retries) != 1:
            errors.append(
                f"{where}: expected exactly one guarded install and one retry, "
                f"found {len(first)} install(s) with id 'doppler_install' and "
                f"{len(retries)} retry step(s) among {len(installs)} usage(s) of "
                f"{ACTION}*"
            )
            continue

        (first_index, first_step), (retry_index, retry_step) = first[0], retries[0]

        if first_step.get("continue-on-error") is not True:
            errors.append(
                f"{where}: the first install must set continue-on-error: true, "
                "otherwise the job dies before the retry can run"
            )

        if retry_step.get("continue-on-error") is not None:
            errors.append(
                f"{where}: the retry must not set continue-on-error — a second "
                "failure has to fail the job, or a real outage reports green"
            )

        if retry_index <= first_index:
            errors.append(f"{where}: the retry step precedes the install it retries")

        if first_step["uses"] != retry_step["uses"]:
            errors.append(
                f"{where}: install and retry pin different refs "
                f"({first_step['uses']} vs {retry_step['uses']})"
            )

        # A skipped first attempt reports outcome 'skipped'. Restating a step's
        # own gate on the retry would let the two drift apart, so the retry is
        # required to carry the outcome check and nothing else.
        if retry_step.get("if") != RETRY_IF:
            errors.append(f"{where}: the retry must be gated on exactly {RETRY_IF!r}")

        guarded += 1

if errors:
    raise SystemExit("\n".join(errors))

if guarded == 0:
    raise SystemExit(
        "no guarded Doppler installs found — this guard has lost its subject, "
        "which means it is silently passing"
    )

print(f"all {guarded} Doppler CLI install(s) bind exactly one fail-closed retry")
PY
