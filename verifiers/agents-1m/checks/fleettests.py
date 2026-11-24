"""L1 fleet tests: real shard, executor and edge code in one process with a
fake control plane, an in-memory store and llmsim (crates/agentd-testkit),
plus the agentd simulations. A file-level point passes when every test in its
file passes; a test-level point passes when the tests whose names contain
its pattern all pass.

Only the files a verifier's points need are run, so one verifier does not
pay for another's tests.
"""

import os

from engine import runners as X
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
}
# (package, file, test-name pattern) -> point
BY_TEST = {}
PACKAGES = {"everruns-agentd-testkit": TESTKIT, "everruns-agentd": AGENTD}

parse = X.cargo_files
parse_tests = X.cargo_tests


def cargo(package, files, log):
    return X.cargo_test(package, files, log)


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
