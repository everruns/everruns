"""Every adapter against a harness output, and the drift guard: each key an
adapter reads must still appear in the harness that prints it."""

import json
import os
import unittest

from checks import cloud, fleettests, local
from engine.config import REPO
from tests.helpers import CFG

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "fixtures", "harness")
SCRIPTS = os.path.join(REPO, "scripts", "agentd")


def fx(name):
    with open(os.path.join(FIX, name)) as f:
        return json.load(f)


def read(path):
    with open(path) as f:
        return f.read()


def vals(out):
    return {e["point"]: e["value"] for e in out.entries}


class Adapters(unittest.TestCase):
    def test_memory_bench_recorded_baseline(self):
        with open(os.path.join(SCRIPTS, "memory_bench_baseline.json")) as f:
            res = json.load(f)
        out = local.memory_bench(res)
        self.assertEqual(vals(out)["C1"], res["gated"]["node_rss_mb_per_1k_sessions"])
        self.assertEqual(vals(out)["C5"], res["gated"]["puts_per_turn"])
        self.assertEqual(out.density, res["sessions"])
        self.assertIn("node_base_mb", out.metrics)

    def test_shard_soak(self):
        samples = [json.loads(l) for l in read(os.path.join(FIX, "shard_soak.samples.jsonl")).splitlines()]
        v = vals(local.shard_soak(fx("shard_soak.summary.json"), samples))
        self.assertEqual(v["A4"], 0)
        self.assertAlmostEqual(v["C3"], 6950 / 10480, places=3)
        self.assertTrue(v["E1"])
        self.assertLess(v["F1"], 0.05)  # 0.05 MB/s on ~9.2 GB
        self.assertGreater(v["F1"], 0.0)

    def test_bench(self):
        out = local.bench(fx("bench.json"))
        self.assertAlmostEqual(vals(out)["C4"], 9.8 + 2 * 3.1)
        self.assertEqual(out.metrics["executor_core_ms_per_turn"], 21.4)

    def test_scale_fits_a_curve(self):
        out = local.scale(fx("scale.json"), goal_shards=40)
        self.assertEqual(len(out.metrics["scale_curve"]), 4)
        self.assertIn("D1", vals(out))

    def test_drill(self):
        v = vals(local.drill(fx("drill.json")))
        self.assertTrue(v["E2"])
        self.assertEqual(v["F3"], 10.2)
        bad = fx("drill.json")
        bad["check"]["second"]["ok"] = False
        self.assertFalse(vals(local.drill(bad))["E2"])

    def test_rejoin_and_many_sessions(self):
        self.assertTrue(vals(local.rejoin(fx("rejoin.json")))["E1"])
        ms = fx("many_sessions.json")
        self.assertTrue(vals(local.many_sessions(ms, 2))["A1"])
        ms["errors"] = 3
        self.assertFalse(vals(local.many_sessions(ms, 2))["A1"])

    def test_last_json_skips_progress_rows(self):
        text = '{"t": 1}\nprogress\n{\n  "a": 1,\n  "b": [1, 2]\n}\n'
        self.assertEqual(local.last_json(text), {"a": 1, "b": [1, 2]})

    def test_cargo_output_parse(self):
        text = ("     Running tests/durability.rs (target/debug/deps/durability-1)\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored\n"
                "     Running tests/recovery.rs (target/debug/deps/recovery-2)\n"
                "test result: FAILED. 3 passed; 1 failed; 0 ignored\n")
        got = fleettests.parse(text, ["durability", "recovery", "stream"])
        self.assertEqual(got, {"durability": (1, 0), "recovery": (3, 1)})

    def test_cargo_reads_one_stream(self):
        # cargo prints "Running" on stderr and results on stdout; read
        # separately, every file looked like it never ran (first baseline).
        from unittest import mock
        import subprocess
        seen = {}

        def run(cmd, **kw):
            seen.update(kw)
            return subprocess.CompletedProcess(cmd, 0, stdout=(
                "     Running tests/durability.rs (x)\n"
                "test result: ok. 2 passed; 0 failed\n"))
        with mock.patch.object(fleettests.X.subprocess, "run", run):
            text, code, _ = fleettests.cargo("p", ["durability"], None)
        self.assertEqual(seen.get("stderr"), subprocess.STDOUT)
        self.assertEqual(fleettests.parse(text, ["durability"]), {"durability": (2, 0)})


class Drift(unittest.TestCase):
    """The keys adapters read, by harness. A harness that renames one fails here."""

    KEYS = {
        "memory_bench.py": ["gated", "node_rss_mb_per_1k_sessions", "puts_per_turn", "rss_mb", "idle", "sessions", "--no-baseline"],
        "shard_soak.py": ["config", "oom_kills", "crashes", "restarts", "peak_remainder_mb", "--snapshot-every", "peak_rss_mb", "peak_unaccounted_mb", "peak_rss_share_of_limit", "chaos", "killed", "errors", "rss_mb", "samples.jsonl", "summary.json"],
        "bench.py": ["leader_cpu_ms_per_turn", "follower_cpu_ms_per_turn", "executor_cpu_ms_per_turn", "--no-baseline"],
        "scale.py": ["shards", "turns_per_sec"],
        "drill.py": ["ship_lag_s", "rebuild_s", "kill_to_leader_s", "second_messages_lost", "check", "rebuilt", "p99"],
        "prove_rejoin_under_load.py": ["\"ok\"", "caught_up_secs", "errors"],
        "many_sessions.py": ["check", "errors", "sessions"],
    }

    def test_harnesses_still_print_what_adapters_read(self):
        for script, keys in self.KEYS.items():
            src = read(os.path.join(SCRIPTS, script))
            for k in keys:
                self.assertIn(k, src, f"{script} no longer mentions {k}: update checks/local.py")

    def test_fleet_test_files_exist(self):
        for f in fleettests.TESTKIT:
            self.assertTrue(os.path.exists(os.path.join(REPO, "crates", "agentd-testkit", "tests", f + ".rs")), f)
        for f in fleettests.AGENTD:
            self.assertTrue(os.path.exists(os.path.join(REPO, "crates", "agentd", "tests", f + ".rs")), f)


class CloudImport(unittest.TestCase):
    def test_newest_pass(self):
        run = cloud.read_run(os.path.join(HERE, "fixtures", "runs", "45197793040"))
        out = cloud.import_run(run, "45197793040", "L3", 1_000_000)
        v = vals(out)
        self.assertTrue(v["A1"])
        self.assertEqual(v["A4"], 0)
        self.assertTrue(v["E1"])
        self.assertAlmostEqual(v["C1"], 11484 / (79905 / 3) * 1000, places=0)
        self.assertAlmostEqual(v["C5"], (24087 + 947238 + 55) / 725370, places=3)
        self.assertAlmostEqual(v["C4"], 11.22 * 1000 / 218.7, places=0)
        self.assertGreater(v["C3"], 0.5)  # the memory design's finding: most of the node is uncounted
        self.assertLess(v["F3"], 15)
        self.assertEqual(out.agents, 72537)
        self.assertGreater(out.density, 25_000)
        for k in ("turn_seconds_in_flight", "executor_core_ms_per_turn", "executor_mb_per_running_turn"):
            self.assertIn(k, out.metrics)

    def test_dispatch_refuses_without_a_reason_or_approval(self):
        with self.assertRaises(SystemExit):
            cloud.dispatch(CFG, "L3", None, dry_run=True)
        with self.assertRaises(SystemExit):
            cloud.dispatch(CFG, "L4", "confirm 1M", dry_run=True)
        cmd = cloud.dispatch(CFG, "L3", "slice after C1 fix", dry_run=True)
        self.assertIn("inputs[sessions]=125000", cmd)
        self.assertIn("inputs[cells]=5", cmd)


if __name__ == "__main__":
    unittest.main()
