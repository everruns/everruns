"""End to end through the real entry point: the free levels at quick size,
an imported 75k run, the plan, checkpoints and the attempt journal, all
against a temporary results store."""

import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

from engine import cli
from engine import store as T

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
RUN = os.path.join(HERE, "fixtures", "runs", "45197793040")


class EndToEnd(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.saved = (T.RESULTS, T.ATTEMPTS, T.CHECKPOINTS, T.watch_hash, T.changed_since)
        T.RESULTS = os.path.join(self.tmp, "results.jsonl")
        T.ATTEMPTS = os.path.join(self.tmp, "attempts.jsonl")
        T.CHECKPOINTS = os.path.join(self.tmp, "checkpoints")
        # Decision: the fixture run's code counts as the code under test. With
        # real git digests, any change to the code a point watches (agentd)
        # halves the imported run's credit, so this test failed on every PR
        # that touched agentd. Freshness has its own tests in test_scoring.
        T.watch_hash = lambda paths, commit=None: "fixture"
        T.changed_since = lambda commit, paths: False

    def tearDown(self):
        T.RESULTS, T.ATTEMPTS, T.CHECKPOINTS, T.watch_hash, T.changed_since = self.saved
        shutil.rmtree(self.tmp)

    def main(self, *args):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(io.StringIO()):
            code = cli.main(list(args) + ["--out", os.path.join(self.tmp, "ev")])
        return code, buf.getvalue()

    def report(self):
        with open(os.path.join(self.tmp, "ev", "report.json")) as f:
            return json.load(f)

    def test_climb_loop(self):
        code, md = self.main("--import-run", RUN, "--commit", "4e2953d0", "--record")
        self.assertEqual(code, 0)
        self.assertIn("Proven agents 72,537", md)
        rows = T.read_jsonl(T.RESULTS)
        self.assertEqual(len(rows), 1)
        self.assertLess(len(json.dumps(rows[0])), 12_000)  # the committed record stays small
        self.assertEqual(rows[0]["at"], "2026-11-23T15:50:51Z")

        code, md = self.main("--quick", "--record")
        rep = self.report()
        self.assertGreater(rep["score"], 20)
        self.assertIsNotNone(rep["resources"])
        self.assertTrue(rep["climb"])
        self.assertIn("## Climb list", md)
        g6 = next(p for p in rep["points"] if p["id"] == "G6")
        self.assertEqual(g6["best_level"], "L1")
        d8 = next(p for p in rep["points"] if p["id"] == "D8")
        self.assertEqual(d8["best_level"], "L1")

        code, out = self.main("--plan")
        self.assertIn("build the check", out)

        self.main("--checkpoint", "base")
        code, out = self.main("--against", "base")
        self.assertEqual(code, 0, out)

        self.main("--attempt", "smaller snapshots", "--points", "C1", "--kept", "no", "--why", "no change")
        self.assertEqual(T.read_jsonl(T.ATTEMPTS)[0]["points"], ["C1"])
        code, out = self.main("--history")
        self.assertIn("smaller snapshots", out)

    def test_regression_against_a_checkpoint_fails(self):
        self.main("--import-run", RUN, "--commit", "4e2953d0", "--record")
        self.main("--checkpoint", "base")
        rows = T.read_jsonl(T.RESULTS)
        worse = dict(rows[0])
        worse["run"] += "-later"
        worse["at"] = "2026-11-23T19:00:00Z"
        worse["entries"] = [dict(e, value=600.0) if e["point"] == "C1" else e for e in rows[0]["entries"]]
        T.append_jsonl(T.RESULTS, worse)
        code, out = self.main("--against", "base")
        self.assertEqual(code, 1)
        self.assertIn("C1", out)

    def test_entry_point_runs(self):
        r = subprocess.run([sys.executable, os.path.join(ROOT, "validate"), "--help"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0)
        self.assertIn("--plan", r.stdout)


if __name__ == "__main__":
    unittest.main()
