"""L1 fleet tests: real shard, executor and edge code in one process with a
fake control plane, an in-memory store and llmsim (crates/agentd-testkit),
plus the Raft simulation. Each test file proves one point; a point passes
when every test in its file passes.
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
AGENTD = {"raft_simulation": "A3"}

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


def cargo(package, files, log):
    cmd = ["cargo", "test", "-p", package]
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
    return parse(text, files), r.returncode, cmd


def entries(results, mapping, out, command):
    for f, point in mapping.items():
        if f not in results:
            out.add(point, "L1", False, note=f"tests/{f}.rs did not run (build failure?)", evidence=command)
            continue
        passed, failed = results[f]
        out.add(point, "L1", failed == 0 and passed > 0,
                note=f"tests/{f}.rs: {passed} passed, {failed} failed", evidence=command)


def run(ctx):
    out = Output()
    log = os.path.join(ctx.evidence, "fleet-tests.log") if ctx.evidence else None
    res, _, cmd = cargo("everruns-agentd-testkit", list(TESTKIT), log)
    entries(res, TESTKIT, out, " ".join(cmd))
    res, _, cmd = cargo("everruns-agentd", list(AGENTD), log)
    entries(res, AGENTD, out, " ".join(cmd))
    return out


CHECKS = [Check("fleet-tests", "L1", sorted(set(TESTKIT.values()) | set(AGENTD.values())),
                minutes=25, needs=("cargo",),
                command="cargo test -p everruns-agentd-testkit --test durability --test recovery "
                        "--test stream --test executors --test control_plane; "
                        "cargo test -p everruns-agentd --test raft_simulation",
                run=run)]
