#!/usr/bin/env bash
# Stress bench (scripts/stress/) guards, hermetic: no binaries, no infra.
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import os
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace

sys.path.insert(0, "scripts/stress")
import metrics  # noqa: E402
import run as stress  # noqa: E402

ARGS = ["--server-bin", "s", "--worker-bin", "w"]


class Sweep(unittest.TestCase):
    def test_grid_is_the_cartesian_product(self):
        points = stress.sweep_points(["env:A=1,2", "pg:commit_delay=0,50"])
        self.assertEqual(len(points), 4)
        self.assertIn([("env", "A", "2"), ("pg", "commit_delay", "50")], points)

    def test_no_sweep_is_one_baseline_point(self):
        self.assertEqual(stress.sweep_points([]), [[]])
        self.assertEqual(stress.label([]), "baseline")

    def test_bad_axis_is_rejected(self):
        for bad in ("A=1,2", "cfg:A=1", "env:A", "env:=1"):
            with self.assertRaises(SystemExit, msg=bad):
                stress.sweep_points([bad])

    def test_point_overrides_keep_argument_types(self):
        args = stress.parse_args(ARGS + ["--env", "X=1", "--pg", "commit_delay=0"])
        a, env, pg = stress.apply_point(args, [("arg", "servers", "1"), ("arg", "fsync-delay-ms", "2.5"),
                                               ("env", "X", "2"), ("pg", "commit_delay", "100")])
        self.assertEqual((a.servers, a.fsync_delay_ms), (1, 2.5))
        self.assertEqual(env, {"X": "2"})
        self.assertEqual(pg, {"commit_delay": "100"})
        self.assertEqual(args.servers, 2, "the base arguments stay untouched")

    def test_prod_shaped_splits_cores_by_role(self):
        self.assertEqual(stress.prod_shaped_cpus(4), "postgres=0-0,server=1-2,worker=3-3")
        self.assertEqual(stress.prod_shaped_cpus(16), "postgres=0-3,server=4-11,worker=12-15")
        args = stress.parse_args(ARGS + ["--prod-shaped"])
        self.assertGreater(args.fsync_delay_ms, 0)
        self.assertTrue(args.cpus.startswith("postgres="))

    def test_roles_get_env_and_core_pinning(self):
        server = SimpleNamespace(kind="server", env={}, cmd=["srv"])
        worker = SimpleNamespace(kind="worker", env={}, cmd=["wrk"])
        stress.configure(SimpleNamespace(servers=[server], workers=[worker]), {"K": "v"}, {"server": "1-2"})
        self.assertEqual(server.cmd, ["taskset", "-c", "1-2", "srv"])
        self.assertEqual(worker.cmd, ["wrk"])
        self.assertEqual(worker.env, {"K": "v"})


class Turns(unittest.TestCase):
    ROWS = [
        # session a: one turn, two model calls (300 ms) and a tool (100 ms) in a 1 s turn.
        ["a", "input.message", "1000", "0"],
        ["a", "turn.started", "1200", "0"],
        ["a", "llm.generation", "1400", "200"],
        ["a", "act.completed", "1600", "100"],
        ["a", "llm.generation", "2000", "100"],
        ["a", "turn.completed", "2200", "0"],
        # session b: an unfinished turn from session a must not leak in; this one fails.
        ["b", "input.message", "5000", "0"],
        ["b", "turn.started", "5100", "0"],
        ["b", "llm.generation", "5300", "100"],
        ["b", "turn.failed", "5500", "0"],
    ]

    def test_turns_split_time_into_model_tool_and_overhead(self):
        a, b = metrics.turns(self.ROWS)
        self.assertEqual((a["pickup"], a["turn"], a["calls"], a["overhead"], a["ok"]), (200, 1000, 2, 600, True))
        self.assertEqual((b["pickup"], b["overhead"], b["ok"]), (100, 300, False))

    def test_summary_reports_overhead_per_model_call(self):
        s = metrics.turn_summary(metrics.turns(self.ROWS))
        self.assertEqual((s["turns"], s["failed_turns"], s["model_calls_per_turn"]), (2, 1, 1.5))
        self.assertEqual(s["overhead_per_call_ms_p50"], 300)

    def test_per_turn_divides_counter_deltas(self):
        before = {"commits": 10, "wal_bytes": 0, "wal_syncs": 0, "wal_sync_ms": 0, "statements": 0,
                  "statement_ms": 0, "cpu_server": 1.0}
        after = dict(before, commits=110, wal_bytes=20480, cpu_server=3.0)
        r = metrics.per_turn(before, after, 10)
        self.assertEqual((r["commits_per_turn"], r["wal_kb_per_turn"], r["cpu_server_ms_per_turn"]), (10, 2, 200))

    def test_background_rate_is_subtracted(self):
        idle0 = {"commits": 0.0, "cpu_server": 0.0}
        idle1 = {"commits": 20.0, "cpu_server": 0.2}  # 2 commits/s idle over 10 s
        rate = metrics.background(idle0, idle1, 10)
        before = {"commits": 20.0, "wal_bytes": 0, "wal_syncs": 0, "wal_sync_ms": 0, "statements": 0,
                  "statement_ms": 0, "cpu_server": 0.2}
        after = dict(before, commits=220.0, cpu_server=1.2)
        net = metrics.per_turn(before, after, 10, rate, 50)  # 100 of the 200 commits were background
        self.assertEqual((net["commits_per_turn"], net["cpu_server_ms_per_turn"]), (10, 0))

    def test_own_cpu_time_is_readable(self):
        sum(i * i for i in range(300000))
        self.assertGreater(metrics.proc_cpu_seconds(os.getpid()), 0)
        self.assertEqual(metrics.proc_cpu_seconds(2 ** 22 + 7), 0.0)


class FsyncShim(unittest.TestCase):
    def test_shim_delays_fsync_only_when_asked(self):
        with tempfile.TemporaryDirectory() as d:
            so = os.path.join(d, "fsync_delay.so")
            subprocess.run(["cc", "-shared", "-fPIC", "-O2", "-o", so, "scripts/stress/fsync_delay.c", "-ldl"],
                           check=True)
            probe = ("import os,time,tempfile\nf=tempfile.TemporaryFile()\nt=time.monotonic()\n"
                     "[os.fsync(f.fileno()) for _ in range(5)]\nprint(time.monotonic()-t)")

            def elapsed(delay_us):
                env = dict(os.environ, LD_PRELOAD=so, FSYNC_DELAY_US=str(delay_us))
                return float(subprocess.run([sys.executable, "-c", probe], env=env, check=True,
                                            capture_output=True, text=True).stdout)

            self.assertGreaterEqual(elapsed(40000), 0.2)
            self.assertLess(elapsed(0), 0.2)


unittest.main(argv=["stress"], verbosity=1)
PY
