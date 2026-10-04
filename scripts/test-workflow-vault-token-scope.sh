#!/usr/bin/env bash
# Guard where the Doppler vault token (`secrets.DOPPLER_TOKEN`) may appear in a
# workflow, and what code may run next to it.
#
# The token unlocks the whole vault, not one credential, so it gets a narrower
# boundary than an ordinary secret (EVE-1190):
#
#   1. Step scope only. The token may appear only in the `env:` of a `run:`
#      step. Workflow- or job-level `env:` hands it to every action in the job,
#      and a `uses:` step's `env:`/`with:` hands it to that action's code.
#   2. Immutable actions. Every external action in a job that holds the token is
#      pinned to a full commit SHA. A mutable tag can be repointed; an earlier
#      action can then plant a binary or shell hook that the credentialed
#      `doppler run` step executes.
#   3. Trusted refs. A job that holds the token is gated on a trusted ref
#      (`github.ref == 'refs/heads/main'`, or a release tag for the release
#      workflows that are dispatched on one), as a top-level `&&` conjunct of
#      its `if:`. `workflow_dispatch` otherwise runs any branch a writer picks.

# THREAT[TM-CI-010]: a mutable action or an untrusted ref runs next to the vault
# token. Mitigation: fail the build on any workflow that widens the boundary.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 - "$@" <<'PY'
from pathlib import Path
import re

import yaml

TOKEN = re.compile(r"secrets\.DOPPLER_TOKEN\b")
SHA_PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
TRUSTED_GATES = {
    "github.ref == 'refs/heads/main'",
    "startsWith(github.ref, 'refs/tags/v')",
}


def mentions_token(value):
    return bool(TOKEN.search(yaml.safe_dump(value))) if value is not None else False


def strip_expression(expr):
    expr = str(expr).strip()
    if expr.startswith("${{") and expr.endswith("}}"):
        expr = expr[3:-2].strip()
    return expr


def top_level_split(expr, operator):
    """Split on `operator` outside parentheses and quotes."""
    parts, depth, quote, start, index = [], 0, None, 0, 0
    while index < len(expr):
        char = expr[index]
        if quote:
            if char == quote:
                quote = None
        elif char == "'":
            quote = char
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        elif depth == 0 and expr.startswith(operator, index):
            parts.append(expr[start:index].strip())
            index += len(operator)
            start = index
            continue
        index += 1
    parts.append(expr[start:].strip())
    return parts


def trusted_ref_gate(condition):
    """True when the job cannot run unless the ref is trusted."""
    if condition is False:
        return True
    if condition is None:
        return False
    expr = strip_expression(condition)
    if expr == "false":
        return True
    # A top-level `||` makes every conjunct optional: `A && B || C` runs on C.
    if len(top_level_split(expr, "||")) > 1:
        return False
    return any(part in TRUSTED_GATES for part in top_level_split(expr, "&&"))


def scan(path, text):
    errors = []
    document = yaml.safe_load(text)
    if not isinstance(document, dict):
        return errors

    if mentions_token(document.get("env")):
        errors.append(f"{path}: workflow-level env exposes the vault token to every job")

    for job_name, job in (document.get("jobs") or {}).items():
        where = f"{path}:{job_name}"
        if not isinstance(job, dict):
            continue
        if mentions_token(job.get("env")):
            errors.append(
                f"{where}: job-level env exposes the vault token to every action in "
                "the job; move it to the `env:` of the `run:` step that needs it"
            )
        secrets = job.get("secrets")
        if secrets == "inherit" or mentions_token(secrets):
            errors.append(f"{where}: passes the vault token to a reusable workflow")

        steps = job.get("steps") or []
        credentialed = mentions_token(job.get("env"))
        for index, step in enumerate(steps):
            label = step.get("name") or step.get("uses") or f"step {index + 1}"
            if not mentions_token(step):
                continue
            credentialed = True
            if "uses" in step:
                errors.append(
                    f"{where}: `{label}` hands the vault token to action code; only "
                    "a `run:` step's env may carry it"
                )
            elif mentions_token({k: v for k, v in step.items() if k != "env"}):
                errors.append(
                    f"{where}: `{label}` references the vault token outside its env"
                )
        if not credentialed:
            continue

        for step in steps:
            uses = step.get("uses")
            if not isinstance(uses, str) or uses.startswith("./"):
                continue
            if not SHA_PINNED.match(uses):
                errors.append(
                    f"{where}: `{uses}` runs beside the vault token on a mutable "
                    "ref; pin it to a full commit SHA (`owner/repo@<sha> # vN`)"
                )

        if not trusted_ref_gate(job.get("if")):
            errors.append(
                f"{where}: holds the vault token without a trusted-ref gate; add "
                f"one of {sorted(TRUSTED_GATES)} as a top-level `&&` conjunct of "
                "the job `if:`"
            )
    return errors


PIN = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
GOOD_JOB = f"""
    if: github.ref == 'refs/heads/main' && github.event_name == 'push'
    steps:
      - uses: {PIN}
      - name: Live
        env:
          DOPPLER_TOKEN: ${{{{ secrets.DOPPLER_TOKEN }}}}
        run: doppler run -- cargo test
"""


def workflow(job_body, top=""):
    return f"on: push\n{top}jobs:\n  live:\n    runs-on: ubuntu-latest{job_body}"


FIXTURES = [
    # (name, workflow text, should_fail)
    ("step-scoped-pinned-main-gated", workflow(GOOD_JOB), False),
    ("workflow-level-token", workflow(GOOD_JOB, "env:\n  DOPPLER_TOKEN: ${{ secrets.DOPPLER_TOKEN }}\n"), True),
    ("job-level-token", workflow(GOOD_JOB.replace(
        "    steps:", "    env:\n      DOPPLER_TOKEN: ${{ secrets.DOPPLER_TOKEN }}\n    steps:", 1)), True),
    ("token-into-action", workflow(GOOD_JOB.replace(
        f"      - uses: {PIN}",
        f"      - uses: {PIN}\n        with:\n          token: ${{{{ secrets.DOPPLER_TOKEN }}}}")), True),
    ("mutable-tag-before-credentialed-step", workflow(GOOD_JOB.replace(PIN, "actions/checkout@v7")), True),
    ("mutable-tag-after-credentialed-step", workflow(
        GOOD_JOB + "      - uses: actions/upload-artifact@v7\n"), True),
    ("no-ref-gate", workflow(GOOD_JOB.replace(
        "github.ref == 'refs/heads/main' && ", "")), True),
    ("dispatch-any-ref", workflow(GOOD_JOB.replace(
        "github.ref == 'refs/heads/main' && github.event_name == 'push'",
        "github.event_name == 'push' || github.event_name == 'workflow_dispatch'")), True),
    ("or-bypasses-ref-gate", workflow(GOOD_JOB.replace(
        "github.event_name == 'push'",
        "github.event_name == 'push' || github.event_name == 'workflow_dispatch'")), True),
    ("ref-gate-inside-or-group", workflow(GOOD_JOB.replace(
        "github.ref == 'refs/heads/main' && github.event_name == 'push'",
        "(github.ref == 'refs/heads/main' || inputs.force) && github.event_name == 'push'")), True),
    ("negated-ref-gate", workflow(GOOD_JOB.replace(
        "github.ref == 'refs/heads/main'", "github.ref != 'refs/heads/main'")), True),
    ("ref-gate-wrapped-in-expression", workflow(GOOD_JOB.replace(
        "if: github.ref == 'refs/heads/main' && github.event_name == 'push'",
        "if: ${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}")), False),
    ("release-tag-gate", workflow(GOOD_JOB.replace(
        "github.ref == 'refs/heads/main'", "startsWith(github.ref, 'refs/tags/v')")), False),
    ("disabled-job", workflow(GOOD_JOB.replace(
        "if: github.ref == 'refs/heads/main' && github.event_name == 'push'", "if: false")), False),
    ("inherited-secrets", "on: push\njobs:\n  call:\n    uses: ./.github/workflows/x.yml\n    secrets: inherit\n", True),
    ("uncredentialed-job-may-use-tags", workflow(
        "\n    steps:\n      - uses: actions/checkout@v7\n      - run: cargo test\n"), False),
]


def self_test():
    failures = []
    for name, body, should_fail in FIXTURES:
        caught = bool(scan(f"<fixture {name}>", body))
        if caught != should_fail:
            expected = "an error" if should_fail else "no error"
            failures.append(f"fixture '{name}': expected {expected}, got the opposite")
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"workflow vault-token scope detector: {len(FIXTURES)} fixtures pass")


self_test()

targets = sorted(Path(".github/workflows").glob("*.yml"))
if not targets:
    raise SystemExit(".github/workflows/*.yml matched nothing; the guard would pass vacuously")
errors, credentialed = [], 0
for path in targets:
    text = path.read_text()
    errors.extend(scan(path, text))
    credentialed += bool(TOKEN.search(text))
if errors:
    raise SystemExit("\n".join(errors))
if credentialed == 0:
    raise SystemExit("no workflow references the vault token; this guard has lost its subject")
print(f"vault token is step-scoped, beside pinned actions, on trusted refs ({credentialed} workflows)")
PY
