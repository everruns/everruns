#!/usr/bin/env python3
"""Print open Dependabot and code-scanning alerts as Markdown.

Usage: GH_TOKEN=... scripts/report_security_alerts.py [owner/repo]

A scheduled cloud-agent session cannot read the alert APIs at all, and not for
the reason it looks like. The agent egress proxy rewrites the `Authorization`
header for `api.github.com`, so every request from such a session authenticates
as the session's own GitHub App installation regardless of what it sends — an
invalid token, and no token, both answer 200. Supplying the Doppler PAT
therefore changes nothing; the installation's scopes are the only ones in play,
and they do not include `security_events`. Both endpoints answer 403, so the
security step of the maintenance sweep silently degrades to the lockfile
scanners — which cannot see GitHub-only advisories, triage state, or CodeQL
findings at all.

An Actions job can grant itself `security-events: read` on the built-in
GITHUB_TOKEN with no org or app administration. Running this there and printing
to the job log turns the alerts into something a later session can read back
over the Actions API, which it already has access to.

That covers code scanning but not all of it. Measured on run 32458309940: with
`security-events: read` the job read code-scanning alerts fine and was still
refused Dependabot alerts with 403. So the two halves have different meanings
and must not be collapsed:

* code scanning unreadable -> a real regression, fail the job.
* Dependabot unreadable -> the standing EVE-926 gap, which no permission
  available to a workflow closes, and which no token substitution closes either
  (see above). Report it and carry on; failing here would make the job
  permanently red and therefore ignored.

Exits non-zero only when an endpoint that should be readable is not, never
because alerts exist: unresolvable-but-accepted advisories are normal here
(EVE-890, EVE-896, EVE-913), and failing on them would train the team to
ignore this job.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from collections import Counter

API = os.environ.get("GITHUB_API_URL", "https://api.github.com")

# Ascending order of concern, used for both sorting and summary lines.
SEVERITY_ORDER = ["critical", "high", "moderate", "medium", "low", "warning", "note"]


def severity_rank(severity: str) -> int:
    try:
        return SEVERITY_ORDER.index(severity)
    except ValueError:
        return len(SEVERITY_ORDER)


def get(repo: str, path: str, token: str) -> tuple[int, object]:
    """Return (status, parsed body). Error statuses come back rather than raise
    so a 403 can be reported as a permission regression, not as "no alerts"."""
    request = urllib.request.Request(
        f"{API}/repos/{repo}/{path}",
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "everruns-security-alerts",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, None


def summarize(counts: Counter) -> str:
    ranked = sorted(counts.items(), key=lambda item: severity_rank(item[0]))
    return ", ".join(f"{count} {name}" for name, count in ranked)


def unexpected(status: int, kind: str) -> list[str]:
    return [f"**Unexpected HTTP {status}** reading {kind}."]


# No `permissions:` key grants a workflow token Dependabot alert reads. Only an
# App installation carrying `security_events` can — a PAT cannot help a
# scheduled session either, because the egress proxy discards it. EVE-926.
DEPENDABOT_BLOCKED = (
    "**Unreadable (403).** No permission available to a workflow token grants "
    "this, and a PAT cannot be substituted from a scheduled session — it needs "
    "`security_events` on the GitHub App installation, tracked in EVE-926. "
    "Code-scanning alerts above/below are unaffected."
)


def dependabot_section(repo: str, token: str) -> tuple[list[str], bool]:
    status, alerts = get(repo, "dependabot/alerts?state=open&per_page=100", token)
    if status == 404:
        return [f"_Dependabot alerts are not enabled for `{repo}`._"], True
    if status == 403:
        # Expected in Actions today. Reported, not fatal — see module docstring.
        return [DEPENDABOT_BLOCKED], True
    if status != 200:
        return unexpected(status, "Dependabot alerts"), False
    return format_dependabot(alerts), True


def format_dependabot(alerts: list) -> list[str]:
    if not alerts:
        return ["No open Dependabot alerts."]

    counts = Counter(alert["security_advisory"]["severity"] for alert in alerts)
    lines = [
        f"{len(alerts)} open ({summarize(counts)}).",
        "",
        "| Severity | Package | Ecosystem | Advisory | Manifest |",
        "| --- | --- | --- | --- | --- |",
    ]
    for alert in sorted(
        alerts, key=lambda a: severity_rank(a["security_advisory"]["severity"])
    ):
        advisory = alert["security_advisory"]
        package = alert["dependency"].get("package") or {}
        manifest = alert["dependency"].get("manifest_path", "?")
        lines.append(
            f"| {advisory['severity']} "
            f"| `{package.get('name', '?')}` "
            f"| {package.get('ecosystem', '?')} "
            f"| [{advisory['ghsa_id']}]({alert['html_url']}) "
            f"| `{manifest}` |"
        )
    return lines


def code_scanning_section(repo: str, token: str) -> tuple[list[str], bool]:
    status, alerts = get(repo, "code-scanning/alerts?state=open&per_page=100", token)
    if status == 404:
        # Also what GitHub returns when code scanning has never produced results.
        return [f"_No code-scanning results for `{repo}`._"], True
    if status == 403:
        # A workflow token *can* read these, so 403 means the grant went away.
        return [
            "**Unreadable (403).** The job token lacks `security-events: read` — "
            "this is readable from a workflow, so the permission regressed."
        ], False
    if status != 200:
        return unexpected(status, "code-scanning alerts"), False
    return format_code_scanning(alerts), True


def format_code_scanning(alerts: list) -> list[str]:
    if not alerts:
        return ["No open code-scanning alerts."]

    def severity_of(alert: dict) -> str:
        rule = alert["rule"]
        return rule.get("security_severity_level") or rule.get("severity") or "unknown"

    counts = Counter(severity_of(alert) for alert in alerts)
    lines = [
        f"{len(alerts)} open ({summarize(counts)}).",
        "",
        "| Severity | Rule | Tool | Location |",
        "| --- | --- | --- | --- |",
    ]
    for alert in sorted(alerts, key=lambda a: severity_rank(severity_of(a))):
        location = (alert.get("most_recent_instance") or {}).get("location") or {}
        where = location.get("path", "?")
        if location.get("start_line"):
            where += f":{location['start_line']}"
        lines.append(
            f"| {severity_of(alert)} "
            f"| [{alert['rule'].get('id', '?')}]({alert['html_url']}) "
            f"| {alert.get('tool', {}).get('name', '?')} "
            f"| `{where}` |"
        )
    return lines


def main() -> int:
    repo = (
        sys.argv[1]
        if len(sys.argv) > 1
        else os.environ.get("GITHUB_REPOSITORY", "everruns/everruns")
    )
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        print("GH_TOKEN (or GITHUB_TOKEN) must be set", file=sys.stderr)
        return 2

    dependabot, dependabot_ok = dependabot_section(repo, token)
    code_scanning, code_scanning_ok = code_scanning_section(repo, token)

    print(f"# Security alerts — {repo}")
    print()
    print("## Dependabot alerts")
    print()
    print("\n".join(dependabot))
    print()
    print("## Code-scanning alerts")
    print()
    print("\n".join(code_scanning))

    if not (dependabot_ok and code_scanning_ok):
        print(
            "\nAn endpoint that should be readable was not; see the report above.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
