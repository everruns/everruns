#!/usr/bin/env python3
"""Render live-provider-matrix coverage as Markdown for the job summary.

The `Live Provider Matrix` job passes when every one of its cells was skipped:
a missing API key and an out-of-credits account both skip, both are deliberate,
and both leave the step green (EVE-951). That makes "the providers are covered"
and "nothing reached a provider" look identical in CI, which is how OpenAI
coverage stayed dark for roughly a week in August 2026 without anyone noticing.

This reads the records the matrix cells append to `LLM_MATRIX_COVERAGE_FILE`
(see `ProviderModelConfig::record_outcome`) and reports what the run actually
exercised. It is reporting only: it does not decide whether the job passes, and
it never exits non-zero on thin coverage — that judgement stays with a human
reading the summary, and turning darkness into a hard failure would red main on
a billing condition, which EVE-943 established is a false positive.

Usage: report_live_matrix_coverage.py <coverage-file>
"""

import collections
import sys

# Cell outcomes, as written by `ProviderModelConfig::record_outcome`.
CONFIGURED = "configured"
NO_KEY = "no-key"
SKIP_LIST = "skip-list"
QUOTA = "quota"

# What a cell is reported as, worst-first. A cell records several times per test
# (the `is_none()` guard, then once per `run_live_turn!` attempt), so its
# outcomes are folded by taking the first state in this order that it recorded.
# `quota` outranks `configured` on purpose: such a cell did reach the provider,
# but the billing error short-circuits the turn before any assertion runs, so it
# verified nothing and must not be counted as covered.
PRECEDENCE = [QUOTA, NO_KEY, SKIP_LIST, CONFIGURED]

VERDICT = {
    CONFIGURED: "ran",
    QUOTA: "skipped — out of quota/credits",
    NO_KEY: "skipped — API key not set",
    SKIP_LIST: "skipped — SKIP_LLM_INTEGRATION_TESTS_PROVIDERS",
}


def parse(text):
    """Fold coverage records into {(provider, env_var, model): outcome}.

    Unparseable lines are ignored rather than raising: this report must never be
    the reason a live matrix run fails.
    """
    seen = collections.defaultdict(set)
    for line in text.splitlines():
        fields = line.split("\t")
        if len(fields) != 4:
            continue
        outcome, provider, env_var, model = (f.strip() for f in fields)
        if outcome not in PRECEDENCE:
            continue
        seen[(provider, env_var, model)].add(outcome)

    cells = {}
    for cell, outcomes in seen.items():
        for candidate in PRECEDENCE:
            if candidate in outcomes:
                cells[cell] = candidate
                break
    return cells


def provider_rollup(cells):
    """Per-provider (ran, total) counts, ordered by provider name."""
    totals = collections.Counter()
    ran = collections.Counter()
    for (provider, _env_var, _model), outcome in cells.items():
        totals[provider] += 1
        if outcome == CONFIGURED:
            ran[provider] += 1
    return [(p, ran[p], totals[p]) for p in sorted(totals)]


def format_report(cells):
    lines = ["# Live provider matrix coverage", ""]

    if not cells:
        lines += [
            "**No coverage recorded.** The step produced no cell records at all, so "
            "nothing can be said about which providers were exercised. That is itself "
            "a regression: either no matrix test ran, or the recording is broken.",
            "",
        ]
        return lines

    rollup = provider_rollup(cells)
    total_ran = sum(r for _, r, _ in rollup)
    total_cells = sum(t for _, _, t in rollup)

    if total_ran == 0:
        lines += [
            f"**No provider was exercised.** All {total_cells} cells were skipped, so "
            "this run verified no provider wire behaviour despite reporting success.",
            "",
        ]
    else:
        lines += [f"{total_ran} of {total_cells} cells reached a provider.", ""]

    lines += ["| Provider | Cells run | Total |", "| --- | ---: | ---: |"]
    for provider, ran, total in rollup:
        mark = "" if ran else " ⚠️"
        lines.append(f"| {provider}{mark} | {ran} | {total} |")
    lines.append("")

    dark = [(c, o) for c, o in cells.items() if o != CONFIGURED]
    if not dark:
        lines += ["Every cell ran.", ""]
        return lines

    lines += ["## Cells that did not run", "", "| Provider | Model | Reason |", "| --- | --- | --- |"]
    for (provider, _env_var, model), outcome in sorted(dark):
        lines.append(f"| {provider} | {model} | {VERDICT[outcome]} |")
    lines.append("")
    return lines


def main(argv):
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as handle:
            text = handle.read()
    except OSError:
        # A missing file is the same signal as an empty one: nothing recorded.
        text = ""
    print("\n".join(format_report(parse(text))))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
