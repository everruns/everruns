"""L1 fleet tests: real shard, executor and edge code in one process with a
fake control plane, an in-memory store and llmsim (crates/agentd-testkit),
plus the agentd simulations. A file-level point passes when every test in its
file passes; a test-level point passes when the tests whose names contain
its pattern all pass.

Only the files a validator's points need are run, so one validator does not
pay for another's tests.
"""

import os
import re
import subprocess

from engine.config import REPO
from . import Check, Output

TESTKIT = {  # test file -> point
    "durability": "A1",
    "recovery": "A2",
    "stream": "B5",
    "executors": "E4",
    "control_plane": "E5",
}
AGENTD = {
    "raft_simulation": "A3",
    "protocol_simulation": "U2",
    "snapshot_memory": "U3",
}
# (package, file, test-name pattern) -> point
BY_TEST = {
    ("everruns-agentd-testkit", "recovery", "two_zones"): "U4",
    ("everruns-agentd-testkit", "recovery", "point_in_time_restore"): "U5",
    ("everruns-agentd-testkit", "recovery", "one_org_restore"): "U6",
}
PACKAGES = {"everruns-agentd-testkit": TESTKIT, "everruns-agentd": AGENTD}

ENV = {"CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0"}


def parse(text, files):
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


def parse_tests(text, files):
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


def cargo(package, files, log):
    cmd = ["cargo", "test", "--no-fail-fast", "-p", package]  # every file reports, not just up to the first failure
    for f in files:
        cmd += ["--test", f]
    env = dict(os.environ, **ENV)
    # One stream: cargo prints "Running tests/x.rs" on stderr and the test
    # results on stdout, so reading them separately loses which is which.
    r = subprocess.run(cmd, cwd=REPO, env=env, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, text=True)
    text = r.stdout
    if log:
        with open(log, "a") as f:
            f.write(text)
    return text, r.returncode, cmd


def entries(results, mapping, out, command):
    for f, point in mapping.items():
        if f not in results:
            out.add(point, "L1", False, note=f"tests/{f}.rs did not run (build failure?)", evidence=command)
            continue
        passed, failed = results[f]
        out.add(point, "L1", failed == 0 and passed > 0,
                note=f"tests/{f}.rs: {passed} passed, {failed} failed", evidence=command)


def test_entries(results, mapping, out, command):
    for (_, f, pattern), point in mapping.items():
        got = {n: ok for n, ok in results.get(f, {}).items() if pattern in n}
        if not got:
            out.add(point, "L1", False, note=f"no test matching '{pattern}' ran in tests/{f}.rs", evidence=command)
            continue
        out.add(point, "L1", all(got.values()),
                note=f"tests/{f}.rs '{pattern}': {sum(got.values())} of {len(got)} passed", evidence=command)


def wanted(cfg):
    """{package: (file mapping, by-test mapping)} for the points cfg has."""
    has = cfg.has if cfg is not None else (lambda p: True)
    plan = {}
    for pkg, mapping in PACKAGES.items():
        files = {f: p for f, p in mapping.items() if has(p)}
        tests = {k: p for k, p in BY_TEST.items() if k[0] == pkg and has(p)}
        if files or tests:
            plan[pkg] = (files, tests)
    return plan


def run(ctx):
    out = Output()
    log = os.path.join(ctx.evidence, "fleet-tests.log") if ctx.evidence else None
    for pkg, (files, tests) in wanted(ctx.cfg).items():
        names = sorted(set(files) | {f for _, f, _ in tests})
        text, _, cmd = cargo(pkg, names, log)
        command = " ".join(cmd)
        entries(parse(text, names), files, out, command)
        test_entries(parse_tests(text, names), tests, out, command)
    return out


def command_for(plan=None):
    plan = plan or {pkg: (m, {k: p for k, p in BY_TEST.items() if k[0] == pkg}) for pkg, m in PACKAGES.items()}
    parts = []
    for pkg, (files, tests) in plan.items():
        names = sorted(set(files) | {f for _, f, _ in tests})
        parts.append(f"cargo test --no-fail-fast -p {pkg} " + " ".join(f"--test {n}" for n in names))
    return "; ".join(parts)


CHECKS = [Check("fleet-tests", "L1",
                sorted(set(TESTKIT.values()) | set(AGENTD.values()) | set(BY_TEST.values())),
                minutes=25, needs=("cargo",), command=command_for(), run=run)]
