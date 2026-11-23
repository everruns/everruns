"""Fed the saved artifacts of real 75k runs, the importer gives the verdicts
history gave. Runs 43774871232, 43803384547 and 43827766384 (Nov 16) lost
turns and had out-of-memory kills; 44355318472 and 45179434646 finished
without errors but had kills; 44396952768 and 45197793040 passed the gate."""

import os
import unittest

from checks import cloud
from engine import score as S
from tests.helpers import CFG, fresh

RUNS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures", "runs")
EXPECT = {  # run: (A1, A4 kills, E1)
    "43774871232": (False, 3, False),
    "43803384547": (False, 14, False),
    "43827766384": (False, 6, False),
    "44355318472": (True, 3, True),
    "44396952768": (True, 0, True),
    "45179434646": (True, 5, True),
    "45197793040": (True, 0, True),
}


def imported(rid):
    out = cloud.import_run(cloud.read_run(os.path.join(RUNS, rid)), rid, "L3", 1_000_000)
    run = {"run": rid, "at": "2026-11-23T00:00:00Z", "commit": "x", "fingerprint": CFG.fingerprint,
           "levels": ["L3"], "agents": out.agents, "density": out.density, "entries": out.entries, "invalid": []}
    return out, run


class Backtest(unittest.TestCase):
    def test_verdicts_match_history(self):
        for rid, (a1, kills, e1) in EXPECT.items():
            out, run = imported(rid)
            v = {e["point"]: e["value"] for e in out.entries}
            self.assertEqual((v["A1"], v["A4"], v["E1"]), (a1, kills, e1), rid)
            sts = S.states(CFG, [run], fresh())
            failed = sorted(p for p, st in sts.items() if st.gate_failed)
            want = sorted(([] if a1 else ["A1"]) + (["A4"] if kills else []))
            self.assertEqual(failed, want, rid)
            self.assertEqual(S.proven_agents(CFG, [run]), 0 if want else out.agents, rid)

    def test_store_lag_of_the_old_shipping(self):
        # Before group commit and all-replica acks, Nov 16 runs had no ship
        # gauge in their samples; the new storage keeps it under 15 s.
        out, _ = imported("45197793040")
        self.assertLess({e["point"]: e["value"] for e in out.entries}["F3"], 15)


if __name__ == "__main__":
    unittest.main()
