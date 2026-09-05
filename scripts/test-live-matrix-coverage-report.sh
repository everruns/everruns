#!/usr/bin/env bash
# Covers scripts/report_live_matrix_coverage.py: the outcome folding, the
# per-provider rollup, and — the point of the script — that a run which skipped
# every cell is reported as verifying nothing rather than as coverage (EVE-951).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

python3 - <<'PY'
import sys

sys.path.insert(0, "scripts")

import report_live_matrix_coverage as r

failures = []


def check(name, condition, detail=""):
    if condition:
        print(f"  ok  {name}")
    else:
        print(f"  FAIL {name} {detail}")
        failures.append(name)


def records(*rows):
    return "\n".join("\t".join(row) for row in rows) + "\n"


# --- folding -----------------------------------------------------------------

# A cell records once per `model()` call — the `is_none()` guard plus one per
# `run_live_turn!` attempt — so the same outcome repeats and must fold to one.
cells = r.parse(
    records(
        ("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-haiku-4-5-20251001"),
        ("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-haiku-4-5-20251001"),
        ("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-haiku-4-5-20251001"),
    )
)
check("repeated records fold to one cell", len(cells) == 1, cells)
check(
    "a configured cell reads as run",
    cells[("anthropic", "ANTHROPIC_API_KEY", "claude-haiku-4-5-20251001")] == r.CONFIGURED,
)

# The load-bearing case: a cell that had a key (so recorded `configured`) but hit
# a billing error reached the provider and asserted nothing. Counting it as
# covered is the exact false picture this report exists to remove.
cells = r.parse(
    records(
        ("configured", "openai", "OPENAI_API_KEY", "gpt-6-astra"),
        ("quota", "openai", "OPENAI_API_KEY", "gpt-6-astra"),
    )
)
check(
    "quota outranks configured",
    cells[("openai", "OPENAI_API_KEY", "gpt-6-astra")] == r.QUOTA,
    cells,
)

check("unparseable lines are ignored", r.parse("garbage\nalso garbage\n") == {})
check("unknown outcomes are ignored", r.parse(records(("weird", "a", "B", "c"))) == {})
check(
    "short rows are ignored",
    r.parse("configured\tanthropic\tANTHROPIC_API_KEY\n") == {},
)

# --- rollup ------------------------------------------------------------------

cells = r.parse(
    records(
        ("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-haiku-4-5-20251001"),
        ("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-sonnet-5"),
        ("quota", "openai", "OPENAI_API_KEY", "gpt-6-astra"),
        ("no-key", "gemini", "GEMINI_API_KEY", "gemini-2.5-flash"),
    )
)
check(
    "rollup counts run vs total per provider",
    r.provider_rollup(cells) == [("anthropic", 2, 2), ("gemini", 0, 1), ("openai", 0, 1)],
    r.provider_rollup(cells),
)

body = "\n".join(r.format_report(cells))
check("report counts cells that reached a provider", "2 of 4 cells reached a provider." in body, body)
check("a provider with no coverage is flagged", "| openai ⚠️ |" in body, body)
check("a covered provider is not flagged", "| anthropic |" in body, body)
check("quota skips name the billing reason", "out of quota/credits" in body, body)
check("missing-key skips name the key", "API key not set" in body, body)

# --- the silent-green cases --------------------------------------------------

# Every cell skipped: the step is green and this must say so in as many words.
body = "\n".join(
    r.format_report(
        r.parse(
            records(
                ("no-key", "openai", "OPENAI_API_KEY", "gpt-6-astra"),
                ("quota", "anthropic", "ANTHROPIC_API_KEY", "claude-sonnet-5"),
            )
        )
    )
)
check("a fully skipped run reports no provider exercised", "**No provider was exercised.**" in body, body)
check("a fully skipped run gives the cell count", "All 2 cells were skipped" in body, body)

# No records at all is a distinct failure from "all skipped": it means no matrix
# test ran, or the recording itself broke.
body = "\n".join(r.format_report(r.parse("")))
check("an empty file reports no coverage recorded", "**No coverage recorded.**" in body, body)
check("an empty file does not claim cells were skipped", "cells were skipped" not in body, body)

# Full coverage must not carry a scary table of nothing.
body = "\n".join(
    r.format_report(
        r.parse(records(("configured", "anthropic", "ANTHROPIC_API_KEY", "claude-sonnet-5")))
    )
)
check("a fully covered run says so", "Every cell ran." in body, body)
check("a fully covered run has no skip table", "Cells that did not run" not in body, body)
check("a fully covered run is not flagged", "⚠️" not in body, body)

# --- CLI ---------------------------------------------------------------------

# A missing file must read as "nothing recorded", not crash the summary step.
check("a missing file exits 0", r.main(["prog", "/nonexistent/coverage.tsv"]) == 0)
check("a bad invocation exits 2", r.main(["prog"]) == 2)

print()
if failures:
    print(f"{len(failures)} check(s) failed")
    sys.exit(1)
print("all checks passed")
PY
