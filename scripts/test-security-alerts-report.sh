#!/usr/bin/env bash
# Covers scripts/report_security_alerts.py: the Markdown shapes, the severity
# ordering, and — the point of the script — that an unreadable endpoint is
# reported as a permission regression instead of an empty alert list (EVE-923).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

python3 - <<'PY'
import sys

sys.path.insert(0, "scripts")

import report_security_alerts as r

failures = []


def check(name, condition, detail=""):
    if condition:
        print(f"  ok  {name}")
    else:
        print(f"  FAIL {name} {detail}")
        failures.append(name)


# An empty list is a real answer, and must not look like an error.
check(
    "empty dependabot list reads as no alerts",
    r.format_dependabot([]) == ["No open Dependabot alerts."],
)
check(
    "empty code-scanning list reads as no alerts",
    r.format_code_scanning([]) == ["No open code-scanning alerts."],
)


def dependabot_alert(severity, name, ghsa):
    return {
        "security_advisory": {"severity": severity, "ghsa_id": ghsa},
        "dependency": {
            "package": {"name": name, "ecosystem": "npm"},
            "manifest_path": "apps/ui/package.json",
        },
        "html_url": f"https://github.com/everruns/everruns/security/dependabot/{ghsa}",
    }


rows = r.format_dependabot(
    [
        dependabot_alert("low", "left-pad", "GHSA-low"),
        dependabot_alert("critical", "log4j", "GHSA-crit"),
        dependabot_alert("moderate", "nanoid", "GHSA-mod"),
    ]
)
body = "\n".join(rows)

check("dependabot count line", rows[0].startswith("3 open ("), rows[0])
check(
    "dependabot summary is ordered by severity",
    rows[0] == "3 open (1 critical, 1 moderate, 1 low).",
    rows[0],
)
check(
    "dependabot rows are ordered by severity",
    body.index("log4j") < body.index("nanoid") < body.index("left-pad"),
)
check("dependabot renders the advisory link", "GHSA-crit" in body)
check("dependabot renders the manifest", "apps/ui/package.json" in body)


def scanning_alert(level, rule_id, line):
    return {
        "rule": {"id": rule_id, "security_severity_level": level},
        "tool": {"name": "CodeQL"},
        "most_recent_instance": {
            "location": {"path": "crates/server/src/main.rs", "start_line": line}
        },
        "html_url": f"https://github.com/everruns/everruns/security/code-scanning/{rule_id}",
    }


rows = r.format_code_scanning(
    [scanning_alert("low", "rs/low-rule", 10), scanning_alert("high", "rs/high-rule", 20)]
)
body = "\n".join(rows)
check(
    "code-scanning summary is ordered by severity",
    rows[0] == "2 open (1 high, 1 low).",
    rows[0],
)
check(
    "code-scanning rows are ordered by severity",
    body.index("rs/high-rule") < body.index("rs/low-rule"),
)
check("code-scanning renders path and line", "crates/server/src/main.rs:20" in body)

# A rule with no security_severity_level still has to render.
rows = r.format_code_scanning(
    [
        {
            "rule": {"id": "rs/warn", "severity": "warning"},
            "tool": {"name": "CodeQL"},
            "most_recent_instance": {"location": {"path": "a.rs"}},
            "html_url": "https://example.invalid",
        }
    ]
)
check("code-scanning falls back to rule severity", "warning" in "\n".join(rows))

# A 403 must never read as "clean" — but the two endpoints mean different
# things by it, and collapsing them is the mistake this guards against.
# Measured on run 32458309940: a workflow token with `security-events: read`
# reads code-scanning alerts and is still refused Dependabot alerts.
check(
    "blocked Dependabot text points at the tracking issue",
    "EVE-923" in r.DEPENDABOT_BLOCKED,
)
check(
    "blocked Dependabot text does not blame a missing workflow permission",
    "lacks `security-events: read`" not in r.DEPENDABOT_BLOCKED,
    r.DEPENDABOT_BLOCKED,
)
check(
    "unexpected status is reported verbatim",
    "500" in r.unexpected(500, "Dependabot alerts")[0],
)


# The fatal/non-fatal split, driven through the section functions with the
# network stubbed, since that split is the whole contract of this script.
def with_stub(status, body, fn):
    original = r.get
    r.get = lambda repo, path, token: (status, body)
    try:
        return fn("everruns/everruns", "token")
    finally:
        r.get = original


lines, ok = with_stub(403, None, r.dependabot_section)
check("Dependabot 403 is reported but not fatal", ok is True and "EVE-923" in lines[0])

lines, ok = with_stub(403, None, r.code_scanning_section)
check("code-scanning 403 is fatal", ok is False, f"ok={ok}")
check("code-scanning 403 calls out the regression", "regressed" in lines[0], lines[0])

lines, ok = with_stub(200, [], r.dependabot_section)
check("readable-and-empty Dependabot is not fatal", ok is True and "No open" in lines[0])

lines, ok = with_stub(404, None, r.dependabot_section)
check("Dependabot 404 is not fatal", ok is True and "not enabled" in lines[0])

lines, ok = with_stub(500, None, r.code_scanning_section)
check("unexpected code-scanning status is fatal", ok is False and "500" in lines[0])

lines, ok = with_stub(500, None, r.dependabot_section)
check("unexpected Dependabot status is fatal", ok is False and "500" in lines[0])

if failures:
    print(f"\n{len(failures)} check(s) failed")
    sys.exit(1)
print("\nall checks passed")
PY
