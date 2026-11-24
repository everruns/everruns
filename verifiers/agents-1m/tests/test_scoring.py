import unittest

from engine import config as C
from engine import score as S
from tests.helpers import CFG, e, fresh, run

W = CFG.total_weight


class Targets(unittest.TestCase):
    def test_weights_add_to_80_over_23_points(self):
        self.assertEqual(len(CFG.points), 23)
        self.assertEqual(W, 80)

    def test_scores_scale_out_not_per_agent_cost(self):
        # Owner, 2026-11-24: per-agent efficiency scores in agents-75k only.
        ids = {p.id for p in CFG.points}
        self.assertFalse(ids & {"C1", "C4", "C6", "F1", "G1", "B4"})
        self.assertLessEqual({"X2", "X3", "X4", "X6", "X7", "E3"}, ids)

    def test_goal_derived_numbers(self):
        g = CFG.derived()
        self.assertAlmostEqual(g["turns_per_sec"], 2777.78, places=1)
        self.assertEqual(g["shards"], 40)
        self.assertAlmostEqual(CFG.target(CFG.point("B1")), 2777.78 * 1.3, places=0)
        self.assertEqual(CFG.target(CFG.point("X3")), 1000)

    def test_milestone_moves_shared_targets(self):
        b1 = CFG.point("B1")
        self.assertAlmostEqual(CFG.target(b1, size=100_000), CFG.target(b1) / 10)

    def test_expressions_refuse_anything_but_arithmetic(self):
        for bad in ("__import__('os')", "goal.__class__", "open('x')", "[1]"):
            with self.assertRaises((ValueError, AttributeError)):
                C.evaluate(bad, CFG.names())

    def test_fingerprint_ignores_approval_only(self):
        g = dict(CFG.goal)
        g["approval"] = {"full_run": "2026-12-01 approved"}
        self.assertEqual(C.fingerprint(g), CFG.fingerprint)
        g["pace"] = dict(g["pace"], turn_every_s=60)
        self.assertNotEqual(C.fingerprint(g), CFG.fingerprint)


class Closeness(unittest.TestCase):
    def test_at_least_gives_partial_credit(self):
        p = CFG.point("B1")
        self.assertAlmostEqual(S.closeness(p, e("B1", "L0", 2100), 2800), 0.75)
        self.assertEqual(S.closeness(p, e("B1", "L0", 9000), 2800), 1.0)

    def test_at_most(self):
        p = CFG.point("X3")
        self.assertAlmostEqual(S.closeness(p, e("X3", "L2", 1250), 1000), 0.8)

    def test_zero_target_is_pass_fail(self):
        p = CFG.point("A4")
        self.assertEqual(S.closeness(p, e("A4", "L2", 0), 0), 1.0)
        self.assertEqual(S.closeness(p, e("A4", "L2", 1), 0), 0.0)

    def test_noise_takes_the_unfavourable_end(self):
        p = CFG.point("X3")
        ent = e("X3", "L2", 400, samples=[390, 400, 420])
        self.assertAlmostEqual(S.closeness(p, ent, 401), 401 / 420)
        self.assertEqual(S.median_value(ent), 400)

    def test_worked_example_from_the_design(self):
        # X3 moves at 1,250 ms p99 against 1 s, local: 5 x 0.8 x 0.5 = 2.0.
        p = CFG.point("X3")
        m = S.closeness(p, e("X3", "L2", 1250), CFG.target(p))
        self.assertAlmostEqual(5 * m * CFG.certainty("L2"), 2.0, places=2)


class Totals(unittest.TestCase):
    def score(self, runs, changed=False):
        return S.total(CFG, S.states(CFG, runs, fresh(changed)))

    def test_best_level_counts(self):
        r = run([e("E1", "L1", True), e("E1", "L2", True)], density=25_000)
        value, raw, failed = self.score([r])
        self.assertAlmostEqual(value, 100 * 3 * 0.5 / W)
        self.assertEqual(failed, [])

    def test_model_alone_tops_out_at_15(self):
        r = run([e(p.id, "L0", True if p.direction == "pass" else (0 if p.direction == "at_most" else 1e9))
                 for p in CFG.points])
        value, _, _ = self.score([r])
        self.assertAlmostEqual(value, 15.0, places=6)

    def test_failed_gate_caps_at_25_and_newest_wins(self):
        good = [e(p.id, "L3", True if p.direction == "pass" else (0 if p.direction == "at_most" else 1e9))
                for p in CFG.points]
        r1 = run(good, at="2026-11-20T00:00:00Z", agents=1_000_000)
        r2 = run([e("A4", "L2", 2)], at="2026-11-22T00:00:00Z")
        value, raw, failed = self.score([r1, r2])
        self.assertEqual(failed, ["A4"])
        self.assertEqual(value, 25.0)
        self.assertGreater(raw, 25.0)
        r3 = run([e("A4", "L2", 0)], at="2026-11-23T00:00:00Z")
        value, _, failed = self.score([r1, r2, r3])
        self.assertEqual(failed, [])
        self.assertGreater(value, 25.0)

    def test_freshness_halves_on_changed_code(self):
        r = run([e("E1", "L2", True)])
        self.assertAlmostEqual(self.score([r], changed=True)[0], 100 * 3 * 0.5 * 0.5 / W)

    def test_freshness_halves_again_after_30_days(self):
        r = run([e("E1", "L2", True)], at="2026-10-01T00:00:00Z")
        self.assertAlmostEqual(self.score([r], changed=True)[0], 100 * 3 * 0.5 * 0.25 / W)


    def test_freshness_follows_the_watched_files_digest(self):
        r = run([e("E1", "L2", True, watch="d1")])
        same = S.Freshness(current=lambda paths: "d1", now=fresh().now)
        moved = S.Freshness(current=lambda paths: "d2", now=fresh().now)
        self.assertEqual(S.states(CFG, [r], same)["E1"].best.f, 1.0)
        self.assertEqual(S.states(CFG, [r], moved)["E1"].best.f, 0.5)


class Validity(unittest.TestCase):
    def test_other_fingerprint_earns_nothing(self):
        r = run([e("E1", "L2", True)], fingerprint="0000")
        sts = S.states(CFG, [r], fresh())
        self.assertEqual(sts["E1"].credit, 0)
        self.assertIn("fingerprint", sts["E1"].excluded[0][2])

    def test_saturated_driver_invalidates_the_run(self):
        r = run([e("E1", "L2", True), e("D7", "L2", 0.95)])
        self.assertTrue(r["invalid"])
        self.assertEqual(S.states(CFG, [r], fresh())["E1"].credit, 0)

    def test_entries_for_points_moved_to_75k_are_ignored(self):
        r = run([e("C4", "L2", 10.0), e("E1", "L2", True)])
        self.assertNotIn("C4", S.states(CFG, [r], fresh()))
        self.assertAlmostEqual(S.total(CFG, S.states(CFG, [r], fresh()))[0], 100 * 3 * 0.5 / W)

    def test_per_shard_evidence_needs_goal_density(self):
        r = run([e("E1", "L2", True)], density=2_000)
        st = S.states(CFG, [r], fresh())["E1"]
        self.assertEqual(st.credit, 0)
        self.assertIn("density", st.excluded[0][2])

    def test_shared_points_need_a_full_size_run_at_cloud_levels(self):
        r = run([e("B1", "L3", 1e9)], agents=125_000)
        self.assertEqual(S.states(CFG, [r], fresh())["B1"].credit, 0)
        # ...but count at a milestone the run reached.
        self.assertGreater(S.states(CFG, [r], fresh(), size=100_000)["B1"].credit, 0)


class Headlines(unittest.TestCase):
    def test_proven_agents_is_the_largest_cloud_run_with_gates_passed(self):
        ok = run([e("A1", "L3", True), e("A4", "L3", 0)], agents=72_537, levels=["L3"])
        bad = run([e("A1", "L3", True), e("A4", "L3", 3)], agents=90_000, levels=["L3"])
        self.assertEqual(CFG.hooks.proven_agents(CFG, [ok, bad]), 72_537)

    def test_milestones_need_gates_and_targets_met(self):
        gates = [e(g, "L1", True) for g in ("A1", "A2", "A3")] + [e("A4", "L2", 0)]
        r = run(gates + [e("X3", "L2", 800.0)])
        ms = S.milestones(CFG, [r], fresh())
        self.assertEqual(ms["L1"], 1_000_000)  # every gate evidenced at L1 or above, X3 met
        self.assertIsNone(ms["L2"])  # A1-A3 only at L1
        r2 = run(gates + [e("X3", "L2", 1500.0)], at="2026-11-23T13:00:00Z")
        self.assertIsNone(S.milestones(CFG, [r2], fresh())["L1"])  # X3 misses its target


if __name__ == "__main__":
    unittest.main()
