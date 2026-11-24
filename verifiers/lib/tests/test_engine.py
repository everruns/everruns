"""The engine on its own: a verifier with nothing to do with agents
(fixtures/pages-uk) runs, scores, caps on a failed gate and records, with no
code from any other verifier. Run from verifiers/lib:
python3 -m unittest discover -s tests -t .
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

LIB = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(LIB, "tests", "fixtures", "pages-uk")
if LIB not in sys.path:
    sys.path.insert(0, LIB)

from engine import runners  # noqa: E402

MAIN = "import sys; sys.path.insert(0, sys.argv[1]); from engine.cli import main; sys.exit(main(sys.argv[2:]))"


class PagesVerifier(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.root = os.path.join(self.tmp, "pages-uk")
        shutil.copytree(FIXTURE, self.root)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def verify(self, *args):
        env = dict(os.environ, VERIFIER_ROOT=self.root)
        r = subprocess.run([sys.executable, "-c", MAIN, LIB, *args, "--out", os.path.join(self.tmp, "ev")],
                           env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def test_scores_with_no_other_verifier_code(self):
        md = self.verify()
        self.assertIn("# Ukrainian pages verifier", md)
        # about.html is in English: the gate fails, the score is capped.
        self.assertIn("Score 25.0 / 100** (capped from 37.5 by failed gates P1)", md)
        self.assertNotIn("agents", md.lower())
        self.assertNotIn("Milestones", md)

    def test_record_and_fix(self):
        self.verify("--record")
        with open(os.path.join(self.root, "results.jsonl")) as f:
            runs = [json.loads(line) for line in f]
        self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]["score"]["failed_gates"], ["P1"])
        with open(os.path.join(self.root, "site", "about.html"), "w") as f:
            f.write('<html lang="uk"><body>Про нас</body></html>\n')
        self.assertIn("**Score 100.0 / 100**.", self.verify())

    def test_plan_and_json(self):
        self.verify("--plan")
        rep = json.loads(self.verify("--json"))
        self.assertEqual([p["id"] for p in rep["points"]], ["P1", "P2"])
        self.assertEqual(rep["milestones"], {})


class Runners(unittest.TestCase):
    def test_cargo_reads_one_stream(self):
        # cargo prints "Running" on stderr and results on stdout; read
        # separately, every file looked like it never ran (first baseline).
        seen = {}

        def run(cmd, **kw):
            seen.update(kw)
            return subprocess.CompletedProcess(cmd, 0, stdout=(
                "     Running tests/durability.rs (x)\n"
                "test result: ok. 2 passed; 0 failed\n"))
        with mock.patch.object(runners.subprocess, "run", run):
            text, code, _ = runners.cargo_test("p", ["durability"], None)
        self.assertEqual(seen.get("stderr"), subprocess.STDOUT)
        self.assertEqual(runners.cargo_files(text, ["durability"]), {"durability": (2, 0)})

    def test_last_json_skips_progress_rows(self):
        self.assertEqual(runners.last_json('row 1\n{"a": 1}\n'), {"a": 1})
        with self.assertRaises(ValueError):
            runners.last_json("no json")


if __name__ == "__main__":
    unittest.main()


class BrokenCheck(unittest.TestCase):
    def test_a_check_that_breaks_does_not_end_the_run(self):
        from engine import cli
        from engine.checks import Check, Output

        def boom(ctx):
            raise ValueError("no JSON result in the output")

        def fine(ctx):
            out = Output()
            out.add("P1", "L0", 1)
            return out

        checks = [Check("broken", "L0", ["P1"], 0.1, "broken", run=boom),
                  Check("fine", "L0", ["P1"], 0.1, "fine", run=fine)]
        with mock.patch("sys.stderr"):
            outs = cli.run_checks(None, checks)
        self.assertEqual([e["value"] for o in outs for e in o.entries], [1])
