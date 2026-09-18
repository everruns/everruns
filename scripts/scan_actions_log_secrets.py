#!/usr/bin/env python3
"""Find unmasked credentials in GitHub Actions job logs.

Usage: scripts/scan_actions_log_secrets.py LOG [LOG ...]

Exits non-zero when a log carries a credential the Actions log masker did not
redact. Findings name the variable and the line, never the value: this runs in
a workflow whose own log is public, so printing the match would re-leak it.

Why this exists. The masker only knows values that flowed through `secrets.*`.
A secret fetched at runtime (`doppler secrets get`) is invisible to it, so the
runner prints it verbatim in the `env:` block it emits for every later step --
that is how live TypeSafe keys reached a public log in run 35305647248.
`scripts/test-workflow-secret-handling.sh` stops that pattern being written;
this is the backstop for the leaks that guard cannot see, from a vendor error
body to a credential a test echoes on failure.

Two rules, aimed at different halves of the problem:

* env-block: inside a runner-emitted `env:` group, a credential-named entry
  whose value is not `***`. The runner masks what it knows, so an opaque value
  sitting beside a `***` is unmasked by construction. This is the rule that
  catches the incident above.
* prefix: a known credential format anywhere in the log, for values that never
  pass through an env block at all.

False positives are what get a scanner switched off, so the allowlist is
derived rather than maintained: a value committed verbatim in a workflow file
is public already and cannot be a leak (CI's hardcoded `POSTGRES_PASSWORD`,
the throwaway `SECRETS_ENCRYPTION_KEY`). It is keyed on the value, not the
variable name, so the same variable holding anything else still reports.

The blind spot that follows: a real secret committed into a workflow file
allowlists itself here. That case belongs to GitHub secret scanning on
repository content, which this does not duplicate.
"""

# THREAT[TM-CI-009]: a credential can reach a public log by a route the static
# guard cannot see — a vendor error body, a value a failing test echoes.
# Mitigation: read the logs themselves and report what the masker did not redact.
from __future__ import annotations

import math
import re
import sys
from pathlib import Path

CREDENTIAL_WORDS = {
    "KEY", "KEYS", "APIKEY", "TOKEN", "TOKENS", "SECRET", "SECRETS",
    "PASSWORD", "PASSWD", "PASSPHRASE", "CREDENTIAL", "CREDENTIALS", "PAT",
}

# What the runner prints in place of a value it knows to be secret.
MASKED = {"***", "", "null", "none"}

TIMESTAMP = re.compile(r"^\S+Z ")
ENV_ENTRY = re.compile(r"^\s{2}([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$")
UNBROKEN_RUN = re.compile(r"[A-Za-z0-9]{16,}")
VALUE_SHAPE = re.compile(r"[A-Za-z0-9+/=_\-.:]+")

# Formats that are a credential wherever they appear. `sk-` excludes `sk-ant-`
# so an Anthropic key reports once, under its own name.
PREFIX_RULES = [
    ("anthropic", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("openai", re.compile(r"sk-(?!ant-)[A-Za-z0-9_\-]{20,}")),
    ("github-pat", re.compile(r"gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,}")),
    # Doppler tokens carry a config segment: dp.st.<config>.<random>.
    ("doppler", re.compile(r"dp\.(st|pt|sa|ct|scim)\.[A-Za-z0-9_.\-]{20,}")),
    ("slack", re.compile(r"xox[baprs]-[A-Za-z0-9\-]{10,}")),
    ("aws", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("gitlab", re.compile(r"glpat-[A-Za-z0-9_\-]{20,}")),
    ("typesafe", re.compile(r"apikey_[A-Za-z0-9]{24,}")),
    ("everruns", re.compile(r"evr_(pat|a2a|app)_[A-Za-z0-9]{16,}")),
]


def shannon_entropy(value: str) -> float:
    counts = {character: value.count(character) for character in set(value)}
    return -sum(
        (count / len(value)) * math.log2(count / len(value)) for count in counts.values()
    )


def looks_like_a_credential(value: str) -> bool:
    """Separate an opaque credential from a readable setting.

    The discriminator is an unbroken alphanumeric run: a key has one, a
    hyphenated setting name like `debug-ubuntu-latest` does not. Entropy alone
    is not enough -- that string scores 3.35, above any threshold a hex key
    (capped at 4.0) could clear.
    """
    if len(value) < 16 or not VALUE_SHAPE.fullmatch(value):
        return False
    if not UNBROKEN_RUN.search(value):
        return False
    return shannon_entropy(value) >= 3.0


def committed_workflow_values(workflow_dir: Path) -> set[str]:
    """Literal values committed in workflow files; public, so never a leak."""
    values: set[str] = set()
    if not workflow_dir.is_dir():
        return values
    for path in sorted(workflow_dir.glob("*.yml")):
        for line in path.read_text(errors="replace").splitlines():
            entry = re.match(r"^\s+[A-Za-z_][A-Za-z0-9_]*:\s*(.+?)\s*$", line)
            if entry:
                values.add(entry.group(1).strip().strip("\"'"))
    return values


def scan(text: str, allowlist: set[str]) -> list[tuple[int, str, str]]:
    """Return (line_number, rule, variable_name) for each unmasked credential."""
    findings = []
    inside_env_block = False
    for number, raw in enumerate(text.splitlines(), 1):
        line = TIMESTAMP.sub("", raw)
        if line.strip() == "env:":
            inside_env_block = True
            continue
        if inside_env_block:
            entry = ENV_ENTRY.match(line)
            if entry:
                name, value = entry.group(1), entry.group(2).strip()
                credential_named = bool(CREDENTIAL_WORDS & set(name.upper().split("_")))
                if (
                    credential_named
                    and value.lower() not in MASKED
                    and value not in allowlist
                    and looks_like_a_credential(value)
                ):
                    findings.append((number, "env-block", name))
                continue
            inside_env_block = False
        for label, pattern in PREFIX_RULES:
            match = pattern.search(line)
            if match and match.group(0) not in allowlist:
                findings.append((number, f"prefix:{label}", "-"))
                break  # one finding per line; the value is the same one
    return findings


def main(argv: list[str]) -> int:
    paths = [Path(arg) for arg in argv]
    if not paths:
        print("usage: scan_actions_log_secrets.py LOG [LOG ...]", file=sys.stderr)
        return 2
    allowlist = committed_workflow_values(Path(".github/workflows"))
    total = 0
    for path in paths:
        for number, rule, name in scan(path.read_text(errors="replace"), allowlist):
            total += 1
            # Deliberately no value: this output is itself a public log.
            print(f"{path.name}:{number}: unmasked credential [{rule}] {name}")
    scanned = f"{len(paths)} log(s)"
    if total:
        print(
            f"\n{total} unmasked credential(s) across {scanned}. "
            "Rotate the affected secrets, then delete the run logs "
            "(DELETE /repos/{owner}/{repo}/actions/runs/{run_id}/logs). "
            "Values are withheld here because this log is public too.",
            file=sys.stderr,
        )
        return 1
    print(f"no unmasked credentials in {scanned}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
