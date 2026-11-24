"""Helpers a check of any verifier can use: run a command with its output
logged, read the JSON a script prints last, run cargo tests and read their
results per test file and per test.

Only what a verifier outside the current ones would plausibly use belongs
here (owner, 2026-11-24); everything specific stays in the verifier.
"""

import json
import os
import re
import subprocess

from .config import REPO

CARGO_ENV = {"CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0"}


def run_logged(cmd, log, cwd=REPO, env=None):
    """Run cmd; stderr then stdout go to log; return (stdout, return code)."""
    with open(log, "w") as f:
        r = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=f, text=True)
    with open(log, "a") as f:
        f.write(r.stdout)
    return r.stdout, r.returncode


def last_json(text):
    """The last JSON value printed (scripts often print progress rows first)."""
    lines = text.splitlines()
    for i in range(len(lines) - 1, -1, -1):
        if lines[i][:1] in "{[":
            try:
                return json.loads("\n".join(lines[i:]))
            except ValueError:
                continue
    raise ValueError("no JSON result in the output")


def cargo_test(package, files, log=None, cwd=REPO):
    """cargo test for some integration test files of a package; every file
    reports (--no-fail-fast). Returns (output, return code, command)."""
    cmd = ["cargo", "test", "--no-fail-fast", "-p", package]
    for f in files:
        cmd += ["--test", f]
    env = dict(os.environ, **CARGO_ENV)
    # One stream: cargo prints "Running tests/x.rs" on stderr and the test
    # results on stdout, so reading them separately loses which is which.
    r = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if log:
        with open(log, "a") as f:
            f.write(r.stdout)
    return r.stdout, r.returncode, cmd


def cargo_files(text, files):
    """{file: (passed, failed)} from cargo test output."""
    out, cur = {}, None
    for line in text.splitlines():
        m = re.search(r"Running tests/(\w+)\.rs", line)
        if m:
            cur = m.group(1) if m.group(1) in files else None
            continue
        m = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed", line)
        if m and cur:
            out[cur] = (int(m.group(2)), int(m.group(3)))
            cur = None
    return out


def cargo_tests(text, files):
    """{file: {test name: ok}} from cargo test output."""
    out, cur = {}, None
    for line in text.splitlines():
        m = re.search(r"Running tests/(\w+)\.rs", line)
        if m:
            cur = m.group(1) if m.group(1) in files else None
            continue
        m = re.match(r"test (\S+) \.\.\. (ok|FAILED)", line.strip())
        if m and cur:
            out.setdefault(cur, {})[m.group(1)] = m.group(2) == "ok"
    return out
