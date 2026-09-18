#!/usr/bin/env bash
# Guard workflows against writing runtime-fetched credentials into the runner
# environment.
#
# GitHub masks a value in logs only when it flowed through `secrets.*`. A secret
# fetched at runtime (`doppler secrets get`) is unknown to the masker, so once it
# reaches $GITHUB_ENV or $GITHUB_OUTPUT the runner prints it verbatim in the
# `env:` block it emits for every later step -- no `echo` required. That is how
# live TypeSafe keys ended up in a public run log (run 35305647248).
#
# The fix is to keep a fetched secret inside the one process that needs it
# (`doppler run -- <cmd>`), so it never crosses a step boundary. `::add-mask::`
# in the same run block is accepted as a fallback, but it is strictly weaker:
# masking is substring-exact, so a consumer that re-encodes the value defeats it.

# THREAT[TM-CI-009]: a credential fetched at runtime is unknown to the Actions log
# masker, so a step boundary publishes it.
# Mitigation: fail the build on any workflow that writes one to a log-visible surface.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 - "$@" <<'PY'
from pathlib import Path
import re
import sys

# Writes to a surface the runner echoes back into the log.
WRITE = re.compile(r">>\s*\"?\$\{?(GITHUB_ENV|GITHUB_OUTPUT)\}?\"?")
# Assignments on the left of that redirect: NAME=... , including `${{ ... }}=...`.
ASSIGN = re.compile(r"(\$\{\{[^}]*\}\}|[A-Za-z_][A-Za-z0-9_]*)=")
# Ways a credential enters a run block at runtime, invisible to the log masker.
RUNTIME_SECRET = re.compile(r"doppler\s+secrets\s+(get|download)|\$\{\{\s*secrets\.")

CREDENTIAL_WORDS = {
    "KEY", "KEYS", "APIKEY", "TOKEN", "TOKENS", "SECRET", "SECRETS",
    "PASSWORD", "PASSWD", "PASSPHRASE", "CREDENTIAL", "CREDENTIALS", "PAT",
}

FIX = (
    "keep the value in the one process that needs it "
    "(`doppler run -- <cmd>`, or an inline `NAME=\"$(...)\" <cmd>`) instead of "
    "writing it to {surface}; if it genuinely must cross a step boundary, "
    "`echo \"::add-mask::$value\"` in the same run block first"
)


def credential_like(name):
    """True when the variable name reads like a credential."""
    return bool(CREDENTIAL_WORDS & set(name.upper().split("_")))


def run_blocks(lines):
    """Yield [(line_number, text), ...] for every `run:` block in a workflow."""
    index = 0
    while index < len(lines):
        line = lines[index]
        stripped = line.strip()
        match = re.match(r"^(\s*)(?:-\s+)?run:\s*([|>][-+]?)?\s*(.*)$", line)
        if not match or not (stripped.startswith("run:") or stripped.startswith("- run:")):
            index += 1
            continue
        indent = len(line) - len(line.lstrip())
        if match.group(2):  # block scalar: `run: |` / `run: >-`
            body = []
            cursor = index + 1
            while cursor < len(lines):
                nxt = lines[cursor]
                if nxt.strip() and (len(nxt) - len(nxt.lstrip())) <= indent:
                    break
                body.append((cursor + 1, nxt))
                cursor += 1
            yield body
            index = cursor
        else:  # single-line `run: cmd`
            yield [(index + 1, match.group(3))]
            index += 1


def scan(path, text):
    """Report every unmasked credential write to a log-visible surface."""
    errors = []
    for body in run_blocks(text.splitlines()):
        masked = any("::add-mask::" in line for _, line in body)
        if masked:
            continue
        for number, line in body:
            if line.lstrip().startswith("#"):
                continue
            write = WRITE.search(line)
            if not write:
                continue
            surface = f"${write.group(1)}"
            names = ASSIGN.findall(line[: write.start()])
            reason = None
            if RUNTIME_SECRET.search(line):
                reason = "writes a runtime-fetched secret to"
            elif any(name.startswith("${{") for name in names):
                reason = "writes an expression-named value to"
            elif any(credential_like(name) for name in names):
                reason = "writes a credential-named value to"
            if reason:
                errors.append(
                    f"{path}:{number}: {reason} {surface}, "
                    f"where the runner prints it unmasked -- "
                    + FIX.format(surface=surface)
                )
    return errors


FIXTURES = [
    # (name, body, should_fail)
    ("leaked-typesafe-key", """
    steps:
      - name: Fetch
        run: |
          KEY="$(doppler secrets get TYPESAFE_API_KEY --plain)"
          echo "TYPESAFE_API_KEY=$KEY" >> "$GITHUB_ENV"
""", True),
    ("inline-fetch-and-export", """
    steps:
      - run: echo "BRAVE_SEARCH_API_KEY=$(doppler secrets get BRAVE_SEARCH_API_KEY --plain)" >> "$GITHUB_ENV"
""", True),
    ("expression-named-export", """
    steps:
      - name: Export
        run: |
          echo "${{ matrix.required_secret }}=$(doppler secrets get X --plain)" >> "$GITHUB_ENV"
""", True),
    ("github-secret-into-output", """
    steps:
      - run: echo "value=${{ secrets.DOPPLER_TOKEN }}" >> "$GITHUB_OUTPUT"
""", True),
    ("unquoted-redirect", """
    steps:
      - run: echo "REGISTRY_TOKEN=$TOKEN" >> $GITHUB_ENV
""", True),
    ("masked-in-same-block", """
    steps:
      - name: Fetch
        run: |
          KEY="$(doppler secrets get TYPESAFE_API_KEY --plain)"
          echo "::add-mask::$KEY"
          echo "TYPESAFE_API_KEY=$KEY" >> "$GITHUB_ENV"
""", False),
    ("doppler-run-scoped-to-one-command", """
    steps:
      - run: doppler run -- cargo test -p everruns-integrations-brave-search --features integration
""", False),
    ("non-credential-export", """
    steps:
      - run: |
          echo "SERVER_PID=$SERVER_PID" >> $GITHUB_ENV
""", False),
    ("comment-mentioning-the-pattern", """
    steps:
      # Writing it to $GITHUB_ENV instead would print it unmasked.
      - run: |
          # echo "API_KEY=$KEY" >> "$GITHUB_ENV"
          doppler run -- cargo test
""", False),
]


def self_test():
    """The detector is only useful if it still catches the original leak."""
    failures = []
    for name, body, should_fail in FIXTURES:
        caught = bool(scan(f"<fixture {name}>", body))
        if caught != should_fail:
            expected = "an error" if should_fail else "no error"
            failures.append(f"fixture '{name}': expected {expected}, got the opposite")
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"workflow secret-handling detector: {len(FIXTURES)} fixtures pass")


self_test()

errors = []
targets = sorted(Path(".github/workflows").glob("*.yml"))
if not targets:
    raise SystemExit(".github/workflows/*.yml matched nothing; the guard would pass vacuously")
for path in targets:
    errors.extend(scan(path, path.read_text()))

if errors:
    raise SystemExit("\n".join(errors))
print(f"no workflow writes a credential to a log-visible surface ({len(targets)} workflows)")
PY
